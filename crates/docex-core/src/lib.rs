//! docex: work on the content of .docx files (text, tables, images, tracked
//! changes, comments) through Markdown, without disturbing the rest.

pub mod comments;
pub mod docmodel;
pub mod math;
pub mod mdparse;
pub mod model;
pub mod package;
pub mod patch;
pub mod render;
pub mod review;
pub mod revisions;
pub mod styles;
pub mod template;
pub mod xml;

use anyhow::{bail, Context, Result};
use comments::{Comment, Comments};
use model::{Block, Ctx, Note, TextBox};
use package::{Package, Rels};
use patch::{align, block_sim, Align, Carry, Opts, PatchCtx, Splice, Stats};
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
const REL_FOOTNOTES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes";
const CT_FOOTNOTES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";
const REL_NUMBERING: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
const CT_NUMBERING: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";

pub struct Docx {
    pub pkg: Package,
    pub main: String,
    pub dom: Document,
    pub styles: Styles,
    pub rels: Rels,
    pub comments: Comments,
    /// Footnotes and endnotes parts.
    notes: Vec<NotePart>,
    numbering_part: String,
    numbering: Option<Document>,
    numbering_existed: bool,
    dirty: Dirty,
}

struct NotePart {
    /// "footnote" or "endnote"
    kind: &'static str,
    /// Label prefix in Markdown: "" for footnotes, "e" for endnotes.
    prefix: &'static str,
    part: String,
    rels: Rels,
    doc: Option<Document>,
    existed: bool,
    dirty: bool,
    rels_dirty: bool,
}

impl NotePart {
    fn load(pkg: &Package, rels: &Rels, kind: &'static str, prefix: &'static str) -> Result<NotePart> {
        let part = rels
            .find_by_type(&format!("/{kind}s"))
            .map(|r| rels.resolve(r.target))
            .unwrap_or_else(|| format!("word/{kind}s.xml"));
        let doc = if rels.find_by_type(&format!("/{kind}s")).is_some() { pkg.read_xml_opt(&part)? } else { None };
        Ok(NotePart { kind, prefix, rels: Rels::load(pkg, &part)?, existed: doc.is_some(), doc, part, dirty: false, rels_dirty: false })
    }

    /// Index in the root's children of the note with this label.
    fn find(&self, label: &str) -> Option<usize> {
        let id = label.strip_prefix(self.prefix)?;
        self.doc.as_ref()?.root.children.iter().position(|c| {
            matches!(c, Node::Element(e) if e.local() == self.kind && e.attr("w:id") == Some(id))
        })
    }
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
        let notes = vec![NotePart::load(&pkg, &rels, "footnote", "")?, NotePart::load(&pkg, &rels, "endnote", "e")?];
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

    /// Footnotes and endnotes as paragraph projections.
    pub fn notes(&self) -> Vec<Note> {
        let mut out = Vec::new();
        for np in &self.notes {
            let Some(doc) = &np.doc else { continue };
            let cx = Ctx { styles: &self.styles, rels: &np.rels };
            for (k, c) in doc.root.children.iter().enumerate() {
                let Node::Element(e) = c else { continue };
                if e.local() != np.kind || e.attr("w:type").is_some_and(|t| t != "normal") {
                    continue;
                }
                out.push(Note {
                    label: format!("{}{}", np.prefix, e.attr("w:id").unwrap_or("")),
                    paras: model::container_paras(e, &[k], &cx),
                });
            }
        }
        out
    }

    pub fn text_boxes(&self) -> Vec<TextBox> {
        model::text_boxes(self.body(), &Ctx { styles: &self.styles, rels: &self.rels })
    }

    fn render_ctx<'a>(&'a self, view: View, notes: &'a [Note], boxes: &'a [TextBox]) -> RenderCtx<'a> {
        let mut ranged = HashSet::new();
        self.body().visit(&mut |e| {
            if e.is("w:commentRangeEnd") {
                if let Some(id) = e.attr("w:id") {
                    ranged.insert(id.to_string());
                }
            }
        });
        for np in &self.notes {
            if let Some(d) = &np.doc {
                d.root.visit(&mut |e| {
                    if e.is("w:commentRangeEnd") {
                        if let Some(id) = e.attr("w:id") {
                            ranged.insert(id.to_string());
                        }
                    }
                });
            }
        }
        RenderCtx { view, comments: &self.comments, ranged, notes, boxes, ids: false, id_prefix: std::cell::Cell::new("c") }
    }

    fn render_full(&self, view: View) -> render::Rendered {
        let (blocks, notes, boxes) = (self.blocks(), self.notes(), self.text_boxes());
        render::render(&blocks, &self.render_ctx(view, &notes, &boxes))
    }

    pub fn render(&self, view: View) -> String {
        self.render_full(view).md
    }

    /// The review view with ids after each change and comment (`⟦c12⟧`,
    /// `⟦m3⟧`), as used by `docex review` / `resolve`. The markers are
    /// ignored when the text is applied.
    pub fn render_with_ids(&self) -> String {
        let (blocks, notes, boxes) = (self.blocks(), self.notes(), self.text_boxes());
        let mut cx = self.render_ctx(View::Review, &notes, &boxes);
        cx.ids = true;
        render::render(&blocks, &cx).md
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
        let mut visit = |e: &Element| {
            if !e.name.starts_with("w:bookmark") && !e.name.starts_with("w:comment") && !e.name.ends_with("note") {
                if let Some(v) = e.attr("w:id").and_then(|v| v.parse::<u32>().ok()) {
                    max = max.max(v);
                }
            }
        };
        self.dom.root.visit(&mut visit);
        for np in &self.notes {
            if let Some(d) = &np.doc {
                d.root.visit(&mut visit);
            }
        }
        max
    }

    /// Apply an edited Markdown version of this document.
    pub fn apply(&mut self, edited: &str, opts: &Opts) -> Result<Report> {
        self.apply_parsed(mdparse::parse_full(edited), opts)
    }

    /// The document as a JSON-able model for graphical editors.
    pub fn model(&self) -> docmodel::DocModel {
        let (blocks, notes, boxes) = (self.blocks(), self.notes(), self.text_boxes());
        let r = render::render(&blocks, &self.render_ctx(View::Review, &notes, &boxes));
        let parsed = mdparse::parse_full(&r.md);
        let known: HashSet<String> = self.comments.list.iter().map(|c| c.id.clone()).collect();
        docmodel::export(&blocks, &r, &parsed, &notes, &boxes, &known)
    }

    /// Apply an edited model (see [`Docx::model`]).
    pub fn apply_model(&mut self, m: &docmodel::DocModel, opts: &Opts) -> Result<Report> {
        self.apply_parsed(docmodel::import(m), opts)
    }

    /// Apply edited content given as parsed blocks.
    pub fn apply_parsed(&mut self, new: mdparse::Parsed, opts: &Opts) -> Result<Report> {
        let r0 = self.render_full(View::Review);
        let orig = mdparse::parse_full(&r0.md);
        if orig.blocks.len() != r0.map.len() {
            bail!(
                "internal error: the rendered document reparses into {} blocks, expected {} (please report this with the file)",
                orig.blocks.len(),
                r0.map.len()
            );
        }
        let mut carry = Carry {
            next_id: self.max_change_id(),
            next_comment_id: self.comments.next_id(&self.dom.root),
            max_image_emu: self.text_width_emu(),
            ..Default::default()
        };
        let mut numbering = self.numbering.take();

        // 1. Text boxes, patched in place in the body DOM.
        let boxes0 = self.text_boxes();
        for (k, used) in &r0.boxes {
            let b = &boxes0[*k];
            let (Some((_, o)), Some((_, n))) = (orig.boxes.iter().find(|x| x.0 == b.id), new.boxes.iter().find(|x| x.0 == b.id)) else {
                continue;
            };
            let paras: Vec<model::Para> = used.iter().map(|i| b.paras[*i].clone()).collect();
            let Some(container) = element_at(self.body(), &b.targets[0]).cloned() else { continue };
            let mut cx = PatchCtx::new(opts, &self.styles, &mut self.rels, &mut numbering, &[], carry);
            let res = patch::patch_paras(&container, &paras, o, n, &mut cx);
            let (c, rels_changed) = cx.finish();
            carry = c;
            self.dirty.rels |= rels_changed;
            if let Some(newc) = res {
                for t in &b.targets {
                    if let Some(e) = element_at_mut(self.body_mut(), t) {
                        *e = newc.clone();
                    }
                }
                self.dirty.dom = true;
            }
        }
        for (id, _) in &new.boxes {
            if !orig.boxes.iter().any(|x| &x.0 == id) {
                carry.warnings.push(format!("text box {id} doesn't exist; new text boxes can't be created"));
            }
        }

        // 2. New footnotes get their ids before the body references them.
        let labels: HashSet<String> = self.notes().into_iter().map(|n| n.label).collect();
        let mut next_note = self.notes[0]
            .doc
            .as_ref()
            .map(|d| d.root.elements().filter_map(|e| e.attr("w:id")?.parse::<i64>().ok()).max().unwrap_or(0))
            .unwrap_or(0)
            .max(0)
            + 1;
        for (label, _) in &new.notes {
            if !labels.contains(label) && !orig.notes.iter().any(|(l, _)| l == label) {
                carry.note_refs.insert(label.clone(), next_note.to_string());
                next_note += 1;
            }
        }

        // 3. The body.
        let blocks = self.blocks();
        let map = r0.map.clone();
        let orig_blocks = &orig.blocks;
        let new_blocks = &new.blocks;
        let ok: Vec<String> = orig_blocks.iter().map(|b| b.key()).collect();
        let nk: Vec<String> = new_blocks.iter().map(|b| b.key()).collect();
        let ops = align(&ok, &nk, |i, j| block_sim(&orig_blocks[i], &new_blocks[j]));
        let body_el = self.body().clone();
        let mut cx = PatchCtx::new(opts, &self.styles, &mut self.rels, &mut numbering, &blocks, carry);
        let mut sp = Splice::default();
        let mut anchor: Option<usize> = None;
        let mut front: Vec<Node> = Vec::new();
        for op in ops {
            match op {
                Align::Keep(i, _) => anchor = Some(i),
                Align::Modify(i, j) => {
                    let bi = map[i];
                    match (&blocks[bi], &orig_blocks[i], &new_blocks[j]) {
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
                    match (&blocks[bi], &orig_blocks[i]) {
                        (Block::Para(p), mdparse::EBlock::Para(o)) => {
                            if let Some(r) = patch::patch_para(p, o, None, Some(bi), &mut cx) {
                                sp.replace.insert(p.path[0], r);
                            }
                        }
                        (Block::Opaque { .. }, _) if opts.keep_opaque => {}
                        (Block::Table(t), _) if opts.keep_opaque && !t.simple => {}
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
                    let node = match &new_blocks[j] {
                        mdparse::EBlock::Para(e) => patch::new_para(e, near, &mut cx),
                        mdparse::EBlock::Table(rows) => patch::new_table(rows, &mut cx),
                        mdparse::EBlock::Opaque(n) => {
                            if !opts.keep_opaque {
                                cx.warnings.push(format!("placeholder b{n} cannot be copied or moved; ignored"));
                            }
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
        let (mut carry, rels_changed) = cx.finish();
        self.dirty.rels |= rels_changed;
        if !sp.is_empty() {
            sp.apply(&mut self.body_mut().children);
            self.dirty.dom = true;
        }

        // 4. Footnote and endnote text.
        let notes0 = self.notes();
        for (label, used) in &r0.notes {
            let (Some((_, o)), Some((_, n))) = (orig.notes.iter().find(|x| &x.0 == label), new.notes.iter().find(|x| &x.0 == label)) else {
                continue;
            };
            let Some(note) = notes0.iter().find(|x| &x.label == label) else { continue };
            let Some(pi) = self.notes.iter().position(|np| np.find(label).is_some()) else { continue };
            let k = self.notes[pi].find(label).unwrap();
            let np = &mut self.notes[pi];
            let Some(Node::Element(container)) = np.doc.as_ref().unwrap().root.children.get(k).cloned() else { continue };
            let paras: Vec<model::Para> = used.iter().map(|i| note.paras[*i].clone()).collect();
            let mut cx = PatchCtx::new(opts, &self.styles, &mut np.rels, &mut numbering, &[], carry);
            let res = patch::patch_paras(&container, &paras, o, n, &mut cx);
            let (c, rels_changed) = cx.finish();
            carry = c;
            np.rels_dirty |= rels_changed;
            if let Some(newc) = res {
                np.doc.as_mut().unwrap().root.children[k] = newc.into();
                np.dirty = true;
            }
        }
        let used = std::mem::take(&mut carry.used_notes);
        if !used.is_empty() {
            self.ensure_footnotes()?;
            for label in used {
                let id = carry.note_refs[&label].clone();
                let paras = new.notes.iter().find(|x| x.0 == label).map(|x| x.1.clone()).unwrap_or_default();
                let np = &mut self.notes[0];
                let mut cx = PatchCtx::new(opts, &self.styles, &mut np.rels, &mut numbering, &[], carry);
                let el = patch::new_footnote(&id, &paras, &mut cx);
                let (c, rels_changed) = cx.finish();
                carry = c;
                np.rels_dirty |= rels_changed;
                np.doc.as_mut().unwrap().root.children.push(el.into());
                np.dirty = true;
            }
        }
        self.numbering = numbering;
        self.dirty.numbering |= carry.numbering_changed;
        if carry.math_added {
            const M_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
            if self.dom.root.attr("xmlns:m").is_none() {
                self.dom.root.set_attr("xmlns:m", M_NS);
            }
            for np in &mut self.notes {
                if let Some(d) = &mut np.doc {
                    if d.root.attr("xmlns:m").is_none() && np.dirty {
                        d.root.set_attr("xmlns:m", M_NS);
                    }
                }
            }
        }
        let (stats, warnings, new_comments, media) = (carry.stats, carry.warnings, carry.new_comments, carry.media);
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

    /// The footnotes part, created with Word's separator notes if missing.
    fn ensure_footnotes(&mut self) -> Result<()> {
        let np = &mut self.notes[0];
        if np.doc.is_none() {
            np.doc = Some(Document::parse(&format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:footnotes xmlns:w=\"{}\" xmlns:r=\"{}\"><w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:r><w:continuationSeparator/></w:r></w:p></w:footnote></w:footnotes>",
                package::W_NS,
                package::R_NS
            ))?);
        }
        if !np.existed {
            let target = np.part.strip_prefix("word/").unwrap_or(&np.part).to_string();
            self.rels.add(REL_FOOTNOTES, &target, false);
            self.pkg.ensure_content_type(Some(&np.part.clone()), None, CT_FOOTNOTES)?;
            np.existed = true;
            self.dirty.rels = true;
        }
        Ok(())
    }

    /// Tracked changes in the body, then in footnotes and endnotes.
    pub fn changes(&self) -> Vec<Change> {
        let mut v = revisions::list(self.body());
        revisions::annotate(self.body(), &mut v);
        for (pi, np) in self.notes.iter().enumerate() {
            let Some(d) = &np.doc else { continue };
            let mut w = revisions::list(&d.root);
            revisions::annotate(&d.root, &mut w);
            for c in &mut w {
                c.scope = pi + 1;
                c.part = format!("{}s", np.kind);
            }
            v.extend(w);
        }
        for (k, c) in v.iter_mut().enumerate() {
            c.n = k + 1;
        }
        v
    }

    /// Accept or reject the changes containing any of these Word change ids.
    pub fn resolve_wids(&mut self, wids: &[String], accept: bool) -> Result<usize> {
        let ns: Vec<usize> = self.changes().iter().filter(|c| c.wids.iter().any(|w| wids.contains(w))).map(|c| c.n).collect();
        if ns.is_empty() {
            return Ok(0);
        }
        self.resolve_changes(&ns, None, accept)
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
        let mut n = 0;
        let body_sel: Vec<Change> = sel.iter().filter(|c| c.scope == 0).cloned().collect();
        if !body_sel.is_empty() {
            n += revisions::resolve(self.body_mut(), &body_sel, accept);
            revisions::sync_fallbacks(self.body_mut());
            self.dirty.dom = true;
        }
        for (pi, np) in self.notes.iter_mut().enumerate() {
            let part_sel: Vec<Change> = sel.iter().filter(|c| c.scope == pi + 1).cloned().collect();
            if let (false, Some(d)) = (part_sel.is_empty(), &mut np.doc) {
                n += revisions::resolve(&mut d.root, &part_sel, accept);
                np.dirty = true;
            }
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

    /// Anchor a new comment on `quote` (within one paragraph).
    ///
    /// If the quote occurs more than once, `nth` (1-based) picks one;
    /// without it the call fails and lists the matches, so a comment never
    /// lands on the wrong passage. `within` restricts the search to sections
    /// whose heading path contains that text.
    pub fn add_comment_at(&mut self, quote: &str, nth: Option<usize>, within: Option<&str>, text: &str, author: &str, date: &str) -> Result<String> {
        if quote.is_empty() {
            bail!("empty quote");
        }
        let blocks = self.blocks();
        // Paragraphs with their section path.
        let mut paras: Vec<(&model::Para, String)> = Vec::new();
        let mut path: Vec<(u8, String)> = Vec::new();
        for b in &blocks {
            match b {
                Block::Para(p) => {
                    if let styles::Kind::Heading(l) = p.kind {
                        path.retain(|(lv, _)| *lv < l);
                        path.push((l, render::view_text(p, View::Accepted)));
                    }
                    paras.push((p, path.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" › ")));
                }
                Block::Table(t) => {
                    let sec = path.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" › ");
                    for r in &t.rows {
                        for c in &r.cells {
                            paras.extend(c.paras.iter().map(|p| (p, sec.clone())));
                        }
                    }
                }
                _ => {}
            }
        }
        struct Hit<'a> {
            p: &'a model::Para,
            section: String,
            si: usize,
            ei: usize,
            context: String,
        }
        let mut hits: Vec<Hit> = Vec::new();
        for (p, section) in &paras {
            if let Some(w) = within {
                if !section.to_lowercase().contains(&w.to_lowercase()) {
                    continue;
                }
            }
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
            for (pos, _) in s.match_indices(quote) {
                let start = s[..pos].chars().count();
                let end = start + quote.chars().count();
                let from = start.saturating_sub(30);
                let ctx: String = s.chars().skip(from).take(end - from + 30).collect();
                hits.push(Hit { p, section: section.clone(), si: chars[start].0, ei: chars[end - 1].0 + 1, context: ctx });
            }
        }
        let hit = match (hits.len(), nth) {
            (0, _) => bail!("text not found in a single paragraph{}: {quote:?}", within.map(|w| format!(" under \"{w}\"")).unwrap_or_default()),
            (n, Some(k)) if k == 0 || k > n => bail!("--nth {k}: there are {n} matches"),
            (_, Some(k)) => &hits[k - 1],
            (1, None) => &hits[0],
            (n, None) => {
                let list: Vec<String> = hits
                    .iter()
                    .enumerate()
                    .take(20)
                    .map(|(i, h)| format!("  {}. {}…{}…", i + 1, if h.section.is_empty() { String::new() } else { format!("§ {}: ", h.section) }, h.context.replace('\n', " ")))
                    .collect();
                bail!("{quote:?} occurs {n} times; pick one with --nth N (or narrow with --in SECTION):\n{}", list.join("\n"))
            }
        };
        let (si, ei) = (hit.si, hit.ei);
        let id = self.comments.next_id(&self.dom.root);
        let opts = Opts { author: author.to_string(), date: date.to_string(), tracked: false, base_dir: None, keep_opaque: false };
        let mut numbering = None;
        let carry = Carry { next_comment_id: id, ..Default::default() };
        let mut cx = PatchCtx::new(&opts, &self.styles, &mut self.rels, &mut numbering, &[], carry);
        let node = patch::comment_on_items(hit.p, si, ei, text, &mut cx);
        let path = hit.p.path.clone();
        self.replace_para(&path, node);
        self.comments.add(&mut self.pkg, &mut self.rels, id, author, date, text, None)?;
        self.dirty.comments = true;
        self.dirty.rels = true;
        Ok(id.to_string())
    }

    /// Anchor a new comment on the only occurrence of `quote`.
    pub fn add_comment(&mut self, quote: &str, text: &str, author: &str, date: &str) -> Result<String> {
        self.add_comment_at(quote, None, None, text, author, date)
    }

    /// Every tracked change and comment thread, with stable ids and context.
    pub fn review(&self) -> review::Review {
        review::build(&self.model(), &self.changes(), &self.comment_list())
    }

    /// Accept and reject by review id (`c12`, `f3`, ...) in one go. Ids are
    /// Word's own, so the order of decisions doesn't matter.
    pub fn resolve_ids(&mut self, accept: &[String], reject: &[String]) -> Result<(usize, usize)> {
        let rv = self.review();
        let mut plan: Vec<(bool, String, Vec<String>)> = Vec::new();
        let mut unknown = Vec::new();
        for (yes, ids) in [(true, accept), (false, reject)] {
            for id in ids {
                match rv.find(id) {
                    Some(it) if it.kind == "comment" => bail!("{id} is a comment; use `docex comment --resolve`"),
                    Some(it) => {
                        if plan.iter().any(|(y, i, _)| *y != yes && *i == it.id) {
                            bail!("{id} is both accepted and rejected");
                        }
                        plan.push((yes, it.id.clone(), it.wids.iter().map(|w| format!("{}:{w}", it.part)).collect()));
                    }
                    None => unknown.push(id.clone()),
                }
            }
        }
        if !unknown.is_empty() {
            bail!("unknown id(s): {} (see `docex review`)", unknown.join(", "));
        }
        let mut counts = (0, 0);
        for yes in [true, false] {
            let keys: Vec<String> = plan.iter().filter(|(y, _, _)| *y == yes).flat_map(|(_, _, k)| k.clone()).collect();
            if keys.is_empty() {
                continue;
            }
            let ns: Vec<usize> = self
                .changes()
                .iter()
                .filter(|c| c.wids.iter().any(|w| keys.contains(&format!("{}:{w}", c.part))))
                .map(|c| c.n)
                .collect();
            let n = if ns.is_empty() { 0 } else { self.resolve_changes(&ns, None, yes)? };
            if yes {
                counts.0 = n;
            } else {
                counts.1 = n;
            }
        }
        Ok(counts)
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
        d.dom || d.rels || d.comments || d.numbering || self.notes.iter().any(|n| n.dirty || n.rels_dirty)
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
        for np in &mut self.notes {
            if np.dirty {
                if let Some(d) = &np.doc {
                    self.pkg.write_xml(&np.part, d);
                }
                np.dirty = false;
            }
            if np.rels_dirty {
                np.rels.save(&mut self.pkg);
                np.rels_dirty = false;
            }
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
        let report = d.apply(md, &Opts { author: author.to_string(), date, tracked: false, base_dir: Some(base_dir.to_path_buf()), keep_opaque: false })?;
        d.dirty.dom = true;
        Ok((d, report))
    }
}

fn element_at<'a>(root: &'a Element, path: &[usize]) -> Option<&'a Element> {
    let mut cur = root;
    for i in path {
        cur = cur.children.get(*i)?.as_element()?;
    }
    Some(cur)
}

fn element_at_mut<'a>(root: &'a mut Element, path: &[usize]) -> Option<&'a mut Element> {
    let mut cur = root;
    for i in path {
        cur = cur.children.get_mut(*i)?.as_element_mut()?;
    }
    Some(cur)
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
