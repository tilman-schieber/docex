//! docex: work on the content of .docx files (text, tables, images, tracked
//! changes, comments) through Markdown, without disturbing the rest.

pub mod comments;
pub mod mdparse;
pub mod model;
pub mod package;
pub mod patch;
pub mod render;
pub mod revisions;
pub mod styles;
pub mod template;
pub mod xml;

use anyhow::{bail, Context, Result};
use comments::{Comment, Comments};
use model::{Block, Ctx};
use package::{Package, Rels};
use patch::{align, block_sim, Align, Opts, PatchCtx, Splice, Stats};
use render::{RenderCtx, View};
use revisions::Change;
use std::collections::HashSet;
use std::path::Path;
use styles::Styles;
use xml::{Document, Element, Node};

pub use patch::Opts as ApplyOpts;

const REL_SETTINGS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings";
const CT_SETTINGS: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml";
/// Children of w:settings that precede w:trackRevisions in the schema.
const SETTINGS_ORDER: &[&str] = &[
    "w:writeProtection", "w:view", "w:zoom", "w:removePersonalInformation", "w:removeDateAndTime",
    "w:doNotDisplayPageBoundaries", "w:displayBackgroundShape", "w:printPostScriptOverText",
    "w:printFractionalCharacterWidth", "w:printFormsData", "w:embedTrueTypeFonts", "w:embedSystemFonts",
    "w:saveSubsetFonts", "w:saveFormsData", "w:mirrorMargins", "w:alignBordersAndEdges",
    "w:bordersDoNotSurroundHeader", "w:bordersDoNotSurroundFooter", "w:gutterAtTop", "w:hideSpellingErrors",
    "w:hideGrammaticalErrors", "w:activeWritingStyle", "w:proofState", "w:formsDesign", "w:attachedTemplate",
    "w:linkStyles", "w:stylePaneFormatFilter", "w:stylePaneSortMethod", "w:documentType", "w:mailMerge",
    "w:revisionView", "w:trackRevisions",
];
const REL_NUMBERING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
const CT_NUMBERING: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";

pub struct Docx {
    pub pkg: Package,
    pub main: String,
    pub dom: Document,
    pub styles: Styles,
    pub rels: Rels,
    pub comments: Comments,
    /// Footnotes and endnotes: (label, text).
    pub notes: Vec<(String, String)>,
    numbering_part: String,
    numbering: Option<Document>,
    numbering_existed: bool,
    dirty: Dirty,
}

#[derive(Default)]
struct Dirty {
    dom: bool,
    rels: bool,
    comments: bool,
    numbering: bool,
}

#[derive(Debug, Default)]
pub struct Report {
    pub stats: Stats,
    pub warnings: Vec<String>,
    pub comments_added: usize,
}

impl Report {
    pub fn changed(&self) -> bool {
        let s = &self.stats;
        s.insertions + s.deletions + s.formats + s.paragraphs_added + s.paragraphs_removed + s.kind_changes + self.comments_added > 0
    }
}

/// Current UTC time as used in `w:date`.
pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn note_texts(pkg: &Package, rels: &Rels, kind: &str, prefix: &str) -> Result<Vec<(String, String)>> {
    let Some(r) = rels.find_by_type(&format!("/{kind}s")) else { return Ok(Vec::new()) };
    let part = rels.resolve(r.target);
    let Some(doc) = pkg.read_xml_opt(&part)? else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for n in doc.root.elements().filter(|e| e.local() == kind) {
        if n.attr("w:type").is_some_and(|t| t != "normal") {
            continue;
        }
        let mut s = String::new();
        n.visit(&mut |e| match e.name.as_str() {
            "w:t" => s.push_str(&e.text_content()),
            "w:tab" => s.push(' '),
            "w:p" if !s.is_empty() => s.push(' '),
            _ => {}
        });
        out.push((format!("{prefix}{}", n.attr("w:id").unwrap_or("")), s.trim().to_string()));
    }
    Ok(out)
}

impl Docx {
    pub fn open(path: &Path) -> Result<Docx> {
        Docx::from_package(Package::open(path)?)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Docx> {
        Docx::from_package(Package::from_bytes(bytes)?)
    }

    pub fn from_package(pkg: Package) -> Result<Docx> {
        let main = pkg.main_part()?;
        let dom = pkg.read_xml(&main)?;
        if dom.root.child("w:body").is_none() {
            bail!("{main} has no w:body");
        }
        let rels = Rels::load(&pkg, &main)?;
        let styles_doc = match rels.find_by_type("/styles") {
            Some(r) => pkg.read_xml_opt(&rels.resolve(r.target))?,
            None => None,
        };
        let (numbering_part, numbering, numbering_existed) = match rels.find_by_type("/numbering") {
            Some(r) => {
                let p = rels.resolve(r.target);
                let d = pkg.read_xml_opt(&p)?;
                let existed = d.is_some();
                (p, d, existed)
            }
            None => (String::from("word/numbering.xml"), None, false),
        };
        let numbering = match numbering {
            Some(n) => Some(n),
            None => Some(Document::parse(&format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:numbering xmlns:w=\"{}\"></w:numbering>",
                package::W_NS
            ))?),
        };
        let styles = Styles::load(styles_doc.as_ref(), numbering.as_ref());
        let comments = Comments::load(&pkg, &rels)?;
        let mut notes = note_texts(&pkg, &rels, "footnote", "")?;
        notes.extend(note_texts(&pkg, &rels, "endnote", "e")?);
        Ok(Docx { pkg, main, dom, styles, rels, comments, notes, numbering_part, numbering, numbering_existed, dirty: Dirty::default() })
    }

    pub fn body(&self) -> &Element {
        self.dom.root.child("w:body").unwrap()
    }

    fn body_mut(&mut self) -> &mut Element {
        self.dom.root.child_mut("w:body").unwrap()
    }

    pub fn blocks(&self) -> Vec<Block> {
        model::project(self.body(), &Ctx { styles: &self.styles, rels: &self.rels })
    }

    fn render_ctx(&self, view: View) -> RenderCtx<'_> {
        let mut ranged = HashSet::new();
        self.body().visit(&mut |e| {
            if e.is("w:commentRangeEnd") {
                if let Some(id) = e.attr("w:id") {
                    ranged.insert(id.to_string());
                }
            }
        });
        RenderCtx { view, comments: &self.comments, ranged, notes: &self.notes }
    }

    pub fn render(&self, view: View) -> String {
        let blocks = self.blocks();
        render::render(&blocks, &self.render_ctx(view)).0
    }

    /// Usable text width of the (last) section, in EMU.
    fn text_width_emu(&self) -> u64 {
        let sect = self.body().elements().filter(|e| e.is("w:sectPr")).last();
        let twips = |name: &str, attr: &str| -> Option<u64> { sect?.child(name)?.attr(attr)?.parse().ok() };
        let w = twips("w:pgSz", "w:w").unwrap_or(12240);
        let l = twips("w:pgMar", "w:left").unwrap_or(1440);
        let r = twips("w:pgMar", "w:right").unwrap_or(1440);
        w.saturating_sub(l + r).max(1440) * 635
    }

    fn max_change_id(&self) -> u32 {
        let mut max = 0u32;
        self.dom.root.visit(&mut |e| {
            if !e.name.starts_with("w:bookmark") && !e.name.starts_with("w:comment") {
                if let Some(v) = e.attr("w:id").and_then(|v| v.parse::<u32>().ok()) {
                    max = max.max(v);
                }
            }
        });
        max
    }

    /// Apply an edited Markdown version of this document.
    pub fn apply(&mut self, edited: &str, opts: &Opts) -> Result<Report> {
        let blocks = self.blocks();
        let (orig_md, map) = render::render(&blocks, &self.render_ctx(View::Review));
        let orig = mdparse::parse(&orig_md);
        if orig.len() != map.len() {
            bail!(
                "internal error: the rendered document reparses into {} blocks, expected {} (please report this with the file)",
                orig.len(),
                map.len()
            );
        }
        let new = mdparse::parse(edited);
        let ok: Vec<String> = orig.iter().map(|b| b.key()).collect();
        let nk: Vec<String> = new.iter().map(|b| b.key()).collect();
        let ops = align(&ok, &nk, |i, j| block_sim(&orig[i], &new[j]));

        let mut numbering = self.numbering.take();
        let next_id = self.max_change_id();
        let next_comment_id = self.comments.next_id(&self.dom.root);
        let max_image_emu = self.text_width_emu();
        let body_el = self.body().clone();
        let mut cx = PatchCtx {
            opts,
            styles: &self.styles,
            rels: &mut self.rels,
            numbering: &mut numbering,
            blocks: &blocks,
            next_id,
            warnings: Vec::new(),
            rels_changed: false,
            numbering_changed: false,
            stats: Stats::default(),
            next_comment_id,
            new_comments: Vec::new(),
            media: Vec::new(),
            max_image_emu,
        };
        let mut sp = Splice::default();
        let mut anchor: Option<usize> = None;
        let mut front: Vec<Node> = Vec::new();
        for op in ops {
            match op {
                Align::Keep(i, _) => anchor = Some(i),
                Align::Modify(i, j) => {
                    let bi = map[i];
                    match (&blocks[bi], &orig[i], &new[j]) {
                        (Block::Para(p), mdparse::EBlock::Para(o), mdparse::EBlock::Para(n)) => {
                            if let Some(r) = patch::patch_para(p, o, Some(n), Some(bi), &mut cx) {
                                sp.replace.insert(p.path[0], r);
                            }
                        }
                        (Block::Table(t), mdparse::EBlock::Table(o), mdparse::EBlock::Table(n)) => {
                            if let Some(Node::Element(tbl)) = body_el.children.get(t.idx) {
                                if let Some(r) = patch::patch_table(t, tbl, o, n, &mut cx) {
                                    sp.replace.insert(t.idx, vec![r.into()]);
                                }
                            }
                        }
                        _ => cx.warnings.push(format!("block b{} changed type; left unchanged", blocks[bi].body_idx())),
                    }
                    anchor = Some(i);
                }
                Align::Delete(i) => {
                    let bi = map[i];
                    match (&blocks[bi], &orig[i]) {
                        (Block::Para(p), mdparse::EBlock::Para(o)) => {
                            if let Some(r) = patch::patch_para(p, o, None, Some(bi), &mut cx) {
                                sp.replace.insert(p.path[0], r);
                            }
                        }
                        (b, _) => {
                            let idx = b.body_idx();
                            if opts.tracked {
                                if let Some(Node::Element(e)) = body_el.children.get(idx) {
                                    let mut wrap = Element::new("x").with_child(e.clone());
                                    patch::mark_deleted(&mut wrap, &mut cx);
                                    sp.replace.insert(idx, wrap.children);
                                }
                            } else {
                                sp.replace.insert(idx, Vec::new());
                            }
                        }
                    }
                    anchor = Some(i);
                }
                Align::Insert(j) => {
                    let near = anchor.map(|i| map[i]);
                    let node = match &new[j] {
                        mdparse::EBlock::Para(e) => patch::new_para(e, near, &mut cx),
                        mdparse::EBlock::Table(rows) => patch::new_table(rows, &mut cx),
                        mdparse::EBlock::Opaque(n) => {
                            cx.warnings.push(format!("placeholder b{n} cannot be copied or moved; ignored"));
                            continue;
                        }
                    };
                    match anchor {
                        Some(i) => sp.after.entry(blocks[map[i]].body_idx()).or_default().push(node),
                        None => front.push(node),
                    }
                }
            }
        }
        if !front.is_empty() {
            match map.first() {
                Some(bi) => {
                    sp.before.entry(blocks[*bi].body_idx()).or_default().splice(0..0, front);
                }
                None => {
                    let sect = body_el.children.iter().rposition(|c| matches!(c, Node::Element(e) if e.is("w:sectPr")));
                    match sect {
                        Some(s) => {
                            sp.before.insert(s, front);
                        }
                        None => sp.append.extend(front),
                    }
                }
            }
        }
        let stats = cx.stats;
        let warnings = std::mem::take(&mut cx.warnings);
        let new_comments = std::mem::take(&mut cx.new_comments);
        let media = std::mem::take(&mut cx.media);
        let rels_changed = cx.rels_changed;
        let numbering_changed = cx.numbering_changed;
        drop(cx);
        self.numbering = numbering;
        if !sp.is_empty() {
            sp.apply(&mut self.body_mut().children);
            self.dirty.dom = true;
        }
        self.dirty.rels |= rels_changed;
        self.dirty.numbering |= numbering_changed;
        for (part, data) in media {
            let ext = part.rsplit('.').next().unwrap_or("png").to_lowercase();
            let ct = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                _ => "image/png",
            };
            self.pkg.ensure_content_type(None, Some(&ext), ct)?;
            self.pkg.write(&part, data);
            self.dirty.rels = true;
        }
        let comments_added = new_comments.len();
        for (id, text) in new_comments {
            self.comments.add(&mut self.pkg, &mut self.rels, id, &opts.author, &opts.date, &text, None)?;
            self.dirty.comments = true;
            self.dirty.rels = true;
        }
        Ok(Report { stats, warnings, comments_added })
    }

    /// Whether the document has "Track Changes" switched on (Word's setting).
    pub fn track_changes(&self) -> Result<bool> {
        let Some(part) = self.settings_part() else { return Ok(false) };
        let Some(doc) = self.pkg.read_xml_opt(&part)? else { return Ok(false) };
        Ok(doc
            .root
            .child("w:trackRevisions")
            .is_some_and(|e| !matches!(e.attr("w:val"), Some("0") | Some("false") | Some("off"))))
    }

    fn settings_part(&self) -> Option<String> {
        self.rels.find_by_type("/settings").map(|r| self.rels.resolve(r.target))
    }

    /// Switch Word's "Track Changes" setting on or off.
    pub fn set_track_changes(&mut self, on: bool) -> Result<()> {
        let part = match self.settings_part() {
            Some(p) => p,
            None => {
                self.rels.add(REL_SETTINGS, "settings.xml", false);
                self.dirty.rels = true;
                self.pkg.ensure_content_type(Some("word/settings.xml"), None, CT_SETTINGS)?;
                "word/settings.xml".to_string()
            }
        };
        let mut doc = match self.pkg.read_xml_opt(&part)? {
            Some(d) => d,
            None => Document::parse(&format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:settings xmlns:w=\"{}\"></w:settings>",
                package::W_NS
            ))?,
        };
        if on {
            doc.root.insert_ordered(Element::new("w:trackRevisions"), SETTINGS_ORDER);
        } else {
            doc.root.remove_children("w:trackRevisions");
        }
        self.pkg.write_xml(&part, &doc);
        Ok(())
    }

    pub fn changes(&self) -> Vec<Change> {
        let mut v = revisions::list(self.body());
        revisions::annotate(self.body(), &mut v);
        v
    }

    /// Accept or reject changes. `which` empty = all; `author` filters.
    pub fn resolve_changes(&mut self, which: &[usize], author: Option<&str>, accept: bool) -> Result<usize> {
        let all = self.changes();
        for n in which {
            if *n == 0 || *n > all.len() {
                bail!("no change #{n} (document has {})", all.len());
            }
        }
        let sel: Vec<Change> = all
            .into_iter()
            .filter(|c| which.is_empty() || which.contains(&c.n))
            .filter(|c| author.is_none_or(|a| c.author == a))
            .collect();
        let n = revisions::resolve(self.body_mut(), &sel, accept);
        if n > 0 {
            self.dirty.dom = true;
        }
        Ok(n)
    }

    /// Comments with the text they are anchored to.
    pub fn comment_list(&self) -> Vec<Comment> {
        let mut anchors: std::collections::HashMap<String, String> = Default::default();
        let mut open: Vec<String> = Vec::new();
        fn walk(e: &Element, open: &mut Vec<String>, anchors: &mut std::collections::HashMap<String, String>) {
            for c in e.elements() {
                match c.name.as_str() {
                    "w:commentRangeStart" => open.push(c.attr("w:id").unwrap_or("").to_string()),
                    "w:commentRangeEnd" => {
                        let id = c.attr("w:id").unwrap_or("");
                        open.retain(|o| o != id);
                    }
                    "w:t" | "w:delText" => {
                        let t = c.text_content();
                        for o in open.iter() {
                            anchors.entry(o.clone()).or_default().push_str(&t);
                        }
                    }
                    "w:p" => {
                        walk(c, open, anchors);
                        for o in open.iter() {
                            anchors.entry(o.clone()).or_default().push(' ');
                        }
                    }
                    _ => walk(c, open, anchors),
                }
            }
        }
        walk(self.body(), &mut open, &mut anchors);
        self.comments
            .list
            .iter()
            .map(|c| {
                let mut c = c.clone();
                c.anchor = anchors.get(&c.id).map(|s| s.trim().to_string()).unwrap_or_default();
                c
            })
            .collect()
    }

    /// Anchor a new comment on `quote` (first occurrence, within one paragraph).
    pub fn add_comment(&mut self, quote: &str, text: &str, author: &str, date: &str) -> Result<String> {
        let blocks = self.blocks();
        let mut paras: Vec<&model::Para> = Vec::new();
        for b in &blocks {
            match b {
                Block::Para(p) => paras.push(p),
                Block::Table(t) => {
                    for r in &t.rows {
                        for c in &r.cells {
                            paras.extend(c.paras.iter());
                        }
                    }
                }
                _ => {}
            }
        }
        for p in paras {
            let chars: Vec<(usize, char)> = p
                .visible()
                .filter(|(_, i)| p.status(i) != model::Status::Del)
                .filter_map(|(ii, i)| match &i.what {
                    model::What::Char(c) => Some((ii, *c)),
                    model::What::Obj { key, .. } if key == "\t" => Some((ii, '\t')),
                    _ => None,
                })
                .collect();
            let s: String = chars.iter().map(|(_, c)| *c).collect();
            if let Some(pos) = s.find(quote) {
                let start = s[..pos].chars().count();
                let end = start + quote.chars().count();
                if end == start {
                    bail!("empty quote");
                }
                let (si, ei) = (chars[start].0, chars[end - 1].0 + 1);
                let id = self.comments.next_id(&self.dom.root);
                let opts = Opts { author: author.to_string(), date: date.to_string(), tracked: false, base_dir: None };
                let mut numbering = None;
                let mut cx = PatchCtx {
                    opts: &opts,
                    styles: &self.styles,
                    rels: &mut self.rels,
                    numbering: &mut numbering,
                    blocks: &[],
                    next_id: 0,
                    warnings: Vec::new(),
                    rels_changed: false,
                    numbering_changed: false,
                    stats: Stats::default(),
                    next_comment_id: id,
                    new_comments: Vec::new(),
                    media: Vec::new(),
                    max_image_emu: 0,
                };
                let node = patch::comment_on_items(p, si, ei, text, &mut cx);
                let path = p.path.clone();
                self.replace_para(&path, node);
                self.comments.add(&mut self.pkg, &mut self.rels, id, author, date, text, None)?;
                self.dirty.comments = true;
                self.dirty.rels = true;
                return Ok(id.to_string());
            }
        }
        bail!("text not found in a single paragraph: {quote:?}")
    }

    fn replace_para(&mut self, path: &[usize], node: Node) {
        let mut cur = self.body_mut();
        for (k, i) in path.iter().enumerate() {
            if k == path.len() - 1 {
                cur.children[*i] = node;
                break;
            }
            cur = cur.children[*i].as_element_mut().unwrap();
        }
        self.dirty.dom = true;
    }

    pub fn reply_comment(&mut self, parent: &str, text: &str, author: &str, date: &str) -> Result<String> {
        if self.comments.get(parent).is_none() {
            bail!("no comment with id {parent}");
        }
        let id = self.comments.next_id(&self.dom.root);
        let ids = id.to_string();
        let mk = |name: &str| Element::new(name).with_attr("w:id", ids.clone());
        let body = self.body_mut();
        let mut placed_ref = false;
        insert_after_match(body, &|e| e.is("w:commentRangeStart") && e.attr("w:id") == Some(parent), mk("w:commentRangeStart").into());
        insert_after_match(body, &|e| e.is("w:commentRangeEnd") && e.attr("w:id") == Some(parent), mk("w:commentRangeEnd").into());
        let ref_run = Element::new("w:r")
            .with_child(Element::new("w:rPr").with_child(Element::new("w:rStyle").with_attr("w:val", "CommentReference")))
            .with_child(mk("w:commentReference"));
        if insert_after_match(
            body,
            &|e| e.is("w:r") && e.elements().any(|c| c.is("w:commentReference") && c.attr("w:id") == Some(parent)),
            ref_run.clone().into(),
        ) {
            placed_ref = true;
        }
        if !placed_ref {
            insert_after_match(body, &|e| e.is("w:commentRangeEnd") && e.attr("w:id") == Some(&ids), ref_run.into());
        }
        self.comments.add(&mut self.pkg, &mut self.rels, id, author, date, text, Some(parent))?;
        self.dirty.dom = true;
        self.dirty.comments = true;
        self.dirty.rels = true;
        Ok(ids)
    }

    pub fn set_comment_done(&mut self, id: &str, done: bool) -> Result<()> {
        self.comments.set_done(&mut self.pkg, &mut self.rels, id, done)?;
        self.dirty.comments = true;
        self.dirty.rels = true;
        Ok(())
    }

    pub fn delete_comment(&mut self, id: &str) -> Result<()> {
        if self.comments.get(id).is_none() {
            bail!("no comment with id {id}");
        }
        let mut ids = vec![id.to_string()];
        ids.extend(self.comments.replies(id).iter().map(|c| c.id.clone()));
        for i in &ids {
            self.comments.remove(i);
        }
        let body = self.body_mut();
        remove_matching(body, &|e: &Element| {
            (matches!(e.name.as_str(), "w:commentRangeStart" | "w:commentRangeEnd")
                && e.attr("w:id").is_some_and(|v| ids.iter().any(|i| i == v)))
                || (e.is("w:r")
                    && e.elements().any(|c| c.is("w:commentReference") && c.attr("w:id").is_some_and(|v| ids.iter().any(|i| i == v))))
        });
        self.dirty.dom = true;
        self.dirty.comments = true;
        Ok(())
    }

    /// Copy embedded images to `dir`, keeping their paths (e.g. `media/image1.png`).
    pub fn extract_media(&self, dir: &Path) -> Result<usize> {
        let mut n = 0;
        for r in self.rels.iter().filter(|r| r.kind.ends_with("/image") && !r.external) {
            let part = self.rels.resolve(r.target);
            let dest = dir.join(r.target.trim_start_matches('/'));
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, self.pkg.read(&part)?).with_context(|| format!("writing {}", dest.display()))?;
            n += 1;
        }
        Ok(n)
    }

    pub fn is_dirty(&self) -> bool {
        let d = &self.dirty;
        d.dom || d.rels || d.comments || d.numbering
    }

    /// Serialize, writing only the parts that changed.
    pub fn to_bytes(&mut self) -> Result<Vec<u8>> {
        if self.dirty.dom {
            self.pkg.write_xml(&self.main.clone(), &self.dom);
        }
        if self.dirty.numbering {
            if let Some(n) = &self.numbering {
                self.pkg.write_xml(&self.numbering_part.clone(), n);
                if !self.numbering_existed {
                    self.rels.add(REL_NUMBERING, "numbering.xml", false);
                    self.pkg.ensure_content_type(Some(&self.numbering_part.clone()), None, CT_NUMBERING)?;
                    self.numbering_existed = true;
                    self.dirty.rels = true;
                }
            }
        }
        if self.dirty.comments {
            self.comments.save(&mut self.pkg);
        }
        if self.dirty.rels {
            self.rels.save(&mut self.pkg);
        }
        self.dirty = Dirty::default();
        self.pkg.to_bytes()
    }

    /// A new document from Markdown, styled by `template` (a .docx) or the built-in one.
    pub fn new_from_markdown(md: &str, template: Option<&Path>, author: &str, base_dir: &Path) -> Result<(Docx, Report)> {
        let date = now_iso();
        let mut d = match template {
            Some(t) => {
                let mut d = Docx::open(t)?;
                let body = d.body_mut();
                body.children.retain(|c| matches!(c, Node::Element(e) if e.is("w:sectPr")));
                d.dirty.dom = true;
                // Re-open from bytes so projections see the emptied body.
                let bytes = d.to_bytes()?;
                Docx::from_bytes(bytes)?
            }
            None => Docx::from_bytes(template::blank(&date)?)?,
        };
        let report = d.apply(md, &Opts { author: author.to_string(), date, tracked: false, base_dir: Some(base_dir.to_path_buf()) })?;
        d.dirty.dom = true;
        Ok((d, report))
    }
}

fn insert_after_match(e: &mut Element, pred: &dyn Fn(&Element) -> bool, node: Node) -> bool {
    for i in 0..e.children.len() {
        let hit = matches!(&e.children[i], Node::Element(c) if pred(c));
        if hit {
            e.children.insert(i + 1, node);
            return true;
        }
        if let Node::Element(c) = &mut e.children[i] {
            if insert_after_match(c, pred, node.clone()) {
                return true;
            }
        }
    }
    false
}

fn remove_matching(e: &mut Element, pred: &dyn Fn(&Element) -> bool) {
    e.children.retain(|c| !matches!(c, Node::Element(x) if pred(x)));
    for c in e.elements_mut() {
        remove_matching(c, pred);
    }
}
