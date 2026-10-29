//! Turning differences between the rendered original and the edited Markdown
//! into surgical XML changes, as tracked revisions by default.

use crate::comments::text_el;
use crate::mdparse::{atoms_key, EAtom, EBlock, EComment, EPara};
use crate::model::{marks_of, Block, Marks, Para, Status, Table, What, WrapKind};
use crate::package::Rels;
use crate::styles::{Kind, Styles};
use crate::xml::{Document, Element, Node};
use similar::{capture_diff_slices, Algorithm, DiffOp};
use std::collections::HashMap;

pub const PPR_ORDER: &[&str] = &[
    "w:pStyle", "w:keepNext", "w:keepLines", "w:pageBreakBefore", "w:framePr", "w:widowControl",
    "w:numPr", "w:suppressLineNumbers", "w:pBdr", "w:shd", "w:tabs", "w:suppressAutoHyphens",
    "w:kinsoku", "w:wordWrap", "w:overflowPunct", "w:topLinePunct", "w:autoSpaceDE", "w:autoSpaceDN",
    "w:bidi", "w:adjustRightInd", "w:snapToGrid", "w:spacing", "w:ind", "w:contextualSpacing",
    "w:mirrorIndents", "w:suppressOverlap", "w:jc", "w:textDirection", "w:textAlignment",
    "w:textboxTightWrap", "w:outlineLvl", "w:divId", "w:cnfStyle", "w:rPr", "w:sectPr", "w:pPrChange",
];

pub const RPR_ORDER: &[&str] = &[
    "w:ins", "w:del", "w:moveFrom", "w:moveTo", "w:rStyle", "w:rFonts", "w:b", "w:bCs", "w:i", "w:iCs",
    "w:caps", "w:smallCaps", "w:strike", "w:dstrike", "w:outline", "w:shadow", "w:emboss", "w:imprint",
    "w:noProof", "w:snapToGrid", "w:vanish", "w:webHidden", "w:color", "w:spacing", "w:w", "w:kern",
    "w:position", "w:sz", "w:szCs", "w:highlight", "w:u", "w:effect", "w:bdr", "w:shd", "w:fitText",
    "w:vertAlign", "w:rtl", "w:cs", "w:em", "w:lang", "w:eastAsianLayout", "w:specVanish", "w:oMath",
    "w:rPrChange",
];

const TRPR_ORDER: &[&str] = &[
    "w:cnfStyle", "w:divId", "w:gridBefore", "w:gridAfter", "w:wBefore", "w:wAfter", "w:cantSplit",
    "w:trHeight", "w:tblHeader", "w:tblCellSpacing", "w:jc", "w:hidden", "w:ins", "w:del", "w:trPrChange",
];

const REL_HYPERLINK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

pub struct Opts {
    pub author: String,
    pub date: String,
    pub tracked: bool,
    /// Directory that relative image paths in the Markdown are resolved against.
    pub base_dir: Option<std::path::PathBuf>,
    /// Keep blocks docex can't edit (TOC, content controls, complex tables)
    /// even when the new text doesn't mention them, e.g. when the text comes
    /// from another document (`docex diff --into`).
    pub keep_opaque: bool,
}

pub struct PatchCtx<'a> {
    pub opts: &'a Opts,
    pub styles: &'a Styles,
    pub rels: &'a mut Rels,
    pub numbering: &'a mut Option<Document>,
    pub blocks: &'a [Block],
    pub next_id: u32,
    pub warnings: Vec<String>,
    pub rels_changed: bool,
    pub numbering_changed: bool,
    pub stats: Stats,
    pub next_comment_id: u32,
    /// Comments anchored during patching, still to be written to comments.xml.
    pub new_comments: Vec<(u32, String)>,
    /// New media parts to add to the package: (part name, bytes).
    pub media: Vec<(String, Vec<u8>)>,
    pub max_image_emu: u64,
    /// Footnote label -> w:id for footnotes created by this edit.
    pub note_refs: HashMap<String, String>,
    /// Labels of new footnotes actually referenced.
    pub used_notes: Vec<String>,
    /// An equation was written (the part may need the math namespace).
    pub math_added: bool,
}

/// State carried from one patching pass (text boxes, body, notes) to the next.
#[derive(Default)]
pub struct Carry {
    pub next_id: u32,
    pub next_comment_id: u32,
    pub warnings: Vec<String>,
    pub stats: Stats,
    pub new_comments: Vec<(u32, String)>,
    pub media: Vec<(String, Vec<u8>)>,
    pub rels_changed: bool,
    pub numbering_changed: bool,
    pub note_refs: HashMap<String, String>,
    pub used_notes: Vec<String>,
    pub math_added: bool,
    pub max_image_emu: u64,
}

impl<'a> PatchCtx<'a> {
    pub fn new(
        opts: &'a Opts,
        styles: &'a Styles,
        rels: &'a mut Rels,
        numbering: &'a mut Option<Document>,
        blocks: &'a [Block],
        c: Carry,
    ) -> PatchCtx<'a> {
        PatchCtx {
            opts,
            styles,
            rels,
            numbering,
            blocks,
            next_id: c.next_id,
            warnings: c.warnings,
            rels_changed: false,
            numbering_changed: c.numbering_changed,
            stats: c.stats,
            next_comment_id: c.next_comment_id,
            new_comments: c.new_comments,
            media: c.media,
            max_image_emu: c.max_image_emu,
            note_refs: c.note_refs,
            used_notes: c.used_notes,
            math_added: c.math_added,
        }
    }

    /// End this pass. `rels_changed` is reported separately because each pass
    /// may work on a different part's relationships.
    pub fn finish(self) -> (Carry, bool) {
        (
            Carry {
                next_id: self.next_id,
                next_comment_id: self.next_comment_id,
                warnings: self.warnings,
                stats: self.stats,
                new_comments: self.new_comments,
                media: self.media,
                rels_changed: false,
                numbering_changed: self.numbering_changed,
                note_refs: self.note_refs,
                used_notes: self.used_notes,
                math_added: self.math_added,
                max_image_emu: self.max_image_emu,
            },
            self.rels_changed,
        )
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct Stats {
    pub insertions: usize,
    pub deletions: usize,
    pub formats: usize,
    pub paragraphs_added: usize,
    pub paragraphs_removed: usize,
    pub kind_changes: usize,
}

impl PatchCtx<'_> {
    fn id(&mut self) -> String {
        self.next_id += 1;
        self.next_id.to_string()
    }

    fn change_el(&mut self, name: &str) -> Element {
        let id = self.id();
        Element::new(name)
            .with_attr("w:id", id)
            .with_attr("w:author", self.opts.author.clone())
            .with_attr("w:date", self.opts.date.clone())
    }
}

/// Where inserted / replaced nodes go inside a container's children.
#[derive(Default)]
pub struct Splice {
    pub replace: HashMap<usize, Vec<Node>>,
    pub before: HashMap<usize, Vec<Node>>,
    pub after: HashMap<usize, Vec<Node>>,
    pub append: Vec<Node>,
}

impl Splice {
    pub fn is_empty(&self) -> bool {
        self.replace.is_empty() && self.before.is_empty() && self.after.is_empty() && self.append.is_empty()
    }

    pub fn apply(mut self, children: &mut Vec<Node>) {
        let old = std::mem::take(children);
        for (i, n) in old.into_iter().enumerate() {
            if let Some(b) = self.before.remove(&i) {
                children.extend(b);
            }
            match self.replace.remove(&i) {
                Some(r) => children.extend(r),
                None => children.push(n),
            }
            if let Some(a) = self.after.remove(&i) {
                children.extend(a);
            }
        }
        children.extend(self.append);
    }
}

// ---------------------------------------------------------------------------
// Alignment

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    Keep(usize, usize),
    Modify(usize, usize),
    Delete(usize),
    Insert(usize),
}

fn similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    similar::TextDiff::from_chars(a, b).ratio()
}

/// Align two sequences by exact key, pairing near-matches inside changed hunks.
pub fn align<F>(old_keys: &[String], new_keys: &[String], sim: F) -> Vec<Align>
where
    F: Fn(usize, usize) -> f32,
{
    let mut out = Vec::new();
    for op in capture_diff_slices(Algorithm::Patience, old_keys, new_keys) {
        match op {
            DiffOp::Equal { old_index, new_index, len } => {
                for k in 0..len {
                    out.push(Align::Keep(old_index + k, new_index + k));
                }
            }
            DiffOp::Delete { old_index, old_len, .. } => {
                out.extend((old_index..old_index + old_len).map(Align::Delete))
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                out.extend((new_index..new_index + new_len).map(Align::Insert))
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                out.extend(pair_hunk(old_index, old_len, new_index, new_len, &sim))
            }
        }
    }
    out
}

fn pair_hunk<F: Fn(usize, usize) -> f32>(oi: usize, on: usize, ni: usize, nn: usize, sim: &F) -> Vec<Align> {
    const THRESHOLD: f32 = 0.4;
    if on * nn > 40_000 {
        // Too big to score; pair positionally.
        let mut v = Vec::new();
        for k in 0..on.max(nn) {
            match (k < on, k < nn) {
                (true, true) => v.push(Align::Modify(oi + k, ni + k)),
                (true, false) => v.push(Align::Delete(oi + k)),
                _ => v.push(Align::Insert(ni + k)),
            }
        }
        return v;
    }
    let s: Vec<Vec<f32>> = (0..on).map(|i| (0..nn).map(|j| sim(oi + i, ni + j)).collect()).collect();
    let mut best = vec![vec![0f32; nn + 1]; on + 1];
    for i in 1..=on {
        for j in 1..=nn {
            let mut b = best[i - 1][j].max(best[i][j - 1]);
            if s[i - 1][j - 1] >= THRESHOLD {
                b = b.max(best[i - 1][j - 1] + s[i - 1][j - 1]);
            }
            best[i][j] = b;
        }
    }
    let (mut i, mut j) = (on, nn);
    let mut rev = Vec::new();
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && s[i - 1][j - 1] >= THRESHOLD && best[i][j] == best[i - 1][j - 1] + s[i - 1][j - 1] {
            rev.push(Align::Modify(oi + i - 1, ni + j - 1));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || best[i][j] == best[i][j - 1]) {
            rev.push(Align::Insert(ni + j - 1));
            j -= 1;
        } else {
            rev.push(Align::Delete(oi + i - 1));
            i -= 1;
        }
    }
    rev.reverse();
    // Between two pairs, put new blocks right after the preceding kept block
    // and deleted ones after them, so e.g. a new list item stays in its list.
    let mut out = Vec::new();
    let mut del = Vec::new();
    for a in rev {
        match a {
            Align::Delete(_) => del.push(a),
            Align::Insert(_) => out.push(a),
            _ => {
                out.append(&mut del);
                out.push(a);
            }
        }
    }
    out.append(&mut del);
    out
}

pub fn block_sim(a: &EBlock, b: &EBlock) -> f32 {
    match (a, b) {
        (EBlock::Para(_), EBlock::Para(_)) | (EBlock::Table(_), EBlock::Table(_)) => similarity(&a.text(), &b.text()),
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// Inline planning

#[derive(Clone, Debug, PartialEq)]
enum CtxEl {
    Orig(usize),
    NewIns(u32),
    NewDel(u32),
    Link(String),
}

#[derive(Clone, Debug, PartialEq)]
enum RunKey {
    Orig(usize, Option<Marks>),
    New { tmpl: Option<usize>, marks: Marks, link: bool },
}

#[derive(Clone, Debug)]
enum Content {
    Char(char),
    Node(Node),
    NewObj(String),
}

#[derive(Clone, Debug)]
struct Out {
    ctx: Vec<CtxEl>,
    run: Option<RunKey>,
    content: Content,
}

/// Word-ish tokens: runs of alphanumerics, or single other atoms.
fn tokenize(atoms: &[EAtom]) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut i = 0;
    let word = |a: &EAtom| a.key.chars().count() == 1 && a.key.chars().next().is_some_and(char::is_alphanumeric);
    while i < atoms.len() {
        let mut j = i + 1;
        if word(&atoms[i]) {
            while j < atoms.len() && word(&atoms[j]) && atoms[j].st == atoms[i].st && atoms[j].link == atoms[i].link {
                j += 1;
            }
        }
        v.push((i, j));
        i = j;
    }
    v
}

fn token_key(atoms: &[EAtom]) -> String {
    // Marks are left out so formatting-only changes pair up as equal text.
    let mut s = String::new();
    for a in atoms {
        s.push_str(&a.key);
        s.push(match a.st {
            Status::Plain => '\u{4}',
            Status::Ins => '\u{5}',
            Status::Del => '\u{6}',
        });
        if let Some(l) = &a.link {
            s.push_str(l);
            s.push('\u{7}');
        }
    }
    s
}

enum AOp {
    Eq(usize, usize),
    Del(usize),
    Ins(usize),
}

fn atom_ops(orig: &[EAtom], new: &[EAtom]) -> Vec<AOp> {
    let ot = tokenize(orig);
    let nt = tokenize(new);
    let ok: Vec<String> = ot.iter().map(|(a, b)| token_key(&orig[*a..*b])).collect();
    let nk: Vec<String> = nt.iter().map(|(a, b)| token_key(&new[*a..*b])).collect();
    let mut ops = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, &ok, &nk) {
        match op {
            DiffOp::Equal { old_index, new_index, len } => {
                for k in 0..len {
                    let (oa, ob) = ot[old_index + k];
                    let (na, _) = nt[new_index + k];
                    for d in 0..ob - oa {
                        ops.push(AOp::Eq(oa + d, na + d));
                    }
                }
            }
            DiffOp::Delete { old_index, old_len, .. } => {
                for t in &ot[old_index..old_index + old_len] {
                    ops.extend((t.0..t.1).map(AOp::Del));
                }
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                for t in &nt[new_index..new_index + new_len] {
                    ops.extend((t.0..t.1).map(AOp::Ins));
                }
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                for t in &ot[old_index..old_index + old_len] {
                    ops.extend((t.0..t.1).map(AOp::Del));
                }
                for t in &nt[new_index..new_index + new_len] {
                    ops.extend((t.0..t.1).map(AOp::Ins));
                }
            }
        }
    }
    ops
}

/// Map rendered-and-reparsed atoms back to projection items.
pub fn map_atoms(p: &Para, orig: &[EAtom]) -> Vec<Option<usize>> {
    let vis: Vec<usize> = p.visible().map(|(i, _)| i).collect();
    let vk: Vec<String> = vis.iter().map(|i| p.items[*i].key().unwrap()).collect();
    let ok: Vec<&str> = orig.iter().map(|a| a.key.as_str()).collect();
    let vk_ref: Vec<&str> = vk.iter().map(|s| s.as_str()).collect();
    let mut map = vec![None; orig.len()];
    for op in capture_diff_slices(Algorithm::Myers, &ok, &vk_ref) {
        if let DiffOp::Equal { old_index, new_index, len } = op {
            for k in 0..len {
                map[old_index + k] = Some(vis[new_index + k]);
            }
        }
    }
    map
}

struct Planner<'p> {
    p: &'p Para,
    outs: Vec<Out>,
    next: usize,
    last_vis: Option<usize>,
    group: u32,
    last_op: u8,
    changed: bool,
}

impl<'p> Planner<'p> {
    fn keep(&mut self, i: usize, marks: Option<Marks>) {
        let it = &self.p.items[i];
        let content = match &it.what {
            What::Char(c) => Content::Char(*c),
            What::Obj { node, .. } | What::Hidden(node) => Content::Node(node.clone()),
        };
        self.outs.push(Out {
            ctx: it.ctx.iter().map(|w| CtxEl::Orig(*w)).collect(),
            run: it.run.map(|r| RunKey::Orig(r, marks)),
            content,
        });
        if it.is_visible() {
            self.last_vis = Some(i);
        }
    }

    fn flush(&mut self, upto: usize) {
        while self.next < upto {
            self.keep(self.next, None);
            self.next += 1;
        }
    }

    fn group_for(&mut self, op: u8) -> u32 {
        if self.last_op != op {
            self.group += 1;
            self.last_op = op;
        }
        self.group
    }

    fn next_visible(&self) -> Option<usize> {
        (self.next..self.p.items.len()).find(|i| self.p.items[*i].is_visible())
    }
}

/// Plan a paragraph's new item sequence. `new = None` deletes all content.
fn plan(p: &Para, orig: &[EAtom], new: Option<&[EAtom]>, comments: &[EComment], cx: &mut PatchCtx) -> (Vec<Out>, bool) {
    let map = map_atoms(p, orig);
    let cm = CommentMarks::new(comments, new.map_or(0, |n| n.len()), cx);
    let tracked = cx.opts.tracked;
    let me = cx.opts.author.clone();
    let mut pl = Planner { p, outs: Vec::new(), next: 0, last_vis: None, group: 0, last_op: 0, changed: false };
    let ops = match new {
        Some(new) => atom_ops(orig, new),
        None => (0..orig.len()).map(AOp::Del).collect(),
    };
    let new = new.unwrap_or(&[]);
    for op in ops {
        let e_idx = match op {
            AOp::Eq(_, e) | AOp::Ins(e) => Some(e),
            AOp::Del(_) => None,
        };
        if let Some(e) = e_idx {
            if cm.before(e, &mut pl.outs, Some(p)) {
                pl.changed = true;
            }
        }
        plan_op(&mut pl, op, &map, orig, new, tracked, &me, cx);
        if let Some(e) = e_idx {
            if cm.after(e, &mut pl.outs, Some(p)) {
                pl.changed = true;
            }
        }
    }
    pl.flush(p.items.len());
    let changed = pl.changed;
    (pl.outs, changed)
}

#[allow(clippy::too_many_arguments)]
fn plan_op(pl: &mut Planner, op: AOp, map: &[Option<usize>], orig: &[EAtom], new: &[EAtom], tracked: bool, me: &str, cx: &mut PatchCtx) {
    let p = pl.p;
    {
        match op {
            AOp::Eq(r, e) => {
                pl.last_op = 0;
                let Some(ii) = map[r] else { return };
                pl.flush(ii);
                let nm = new[e].marks;
                let changed = orig[r].marks != nm && p.items[ii].run.is_some() && p.status(&p.items[ii]) != Status::Del;
                if changed {
                    pl.changed = true;
                    cx.stats.formats += 1;
                }
                pl.keep(ii, changed.then_some(nm));
                pl.next = ii + 1;
            }
            AOp::Del(r) => {
                let Some(ii) = map[r] else { return };
                pl.flush(ii);
                pl.next = ii + 1;
                let it = &p.items[ii];
                let st = p.status(it);
                match st {
                    Status::Del => pl.keep(ii, None),
                    Status::Ins if !tracked || p.change_author(it) == Some(me) => {
                        pl.changed = true;
                        pl.last_vis = Some(ii);
                    }
                    Status::Plain if !tracked => {
                        pl.changed = true;
                        pl.last_vis = Some(ii);
                        cx.stats.deletions += 1;
                    }
                    _ => {
                        pl.changed = true;
                        cx.stats.deletions += 1;
                        let g = pl.group_for(2);
                        pl.keep(ii, None);
                        pl.outs.last_mut().unwrap().ctx.push(CtxEl::NewDel(g));
                    }
                }
            }
            AOp::Ins(e) => {
                let a = &new[e];
                if a.st == Status::Del {
                    return;
                }
                pl.changed = true;
                cx.stats.insertions += 1;
                let track = tracked || a.st == Status::Ins;
                let tmpl = pl.last_vis.or_else(|| pl.next_visible());
                let g = pl.group_for(1);
                let mut ctx = Vec::new();
                let mut own_ins = false;
                let mut has_link = false;
                if let Some(t) = tmpl {
                    for w in &p.items[t].ctx {
                        let wr = &p.wraps[*w];
                        match wr.kind {
                            WrapKind::Other => ctx.push(CtxEl::Orig(*w)),
                            WrapKind::Hyperlink if a.link.is_some() && a.link.as_deref() == wr.link.as_deref() => {
                                has_link = true;
                                ctx.push(CtxEl::Orig(*w))
                            }
                            WrapKind::Ins if track && wr.author.as_deref() == Some(me) => {
                                own_ins = true;
                                ctx.push(CtxEl::Orig(*w))
                            }
                            _ => {}
                        }
                    }
                }
                if let (Some(l), false) = (&a.link, has_link) {
                    let pos = ctx.iter().position(|c| matches!(c, CtxEl::Orig(w) if p.wraps[*w].kind == WrapKind::Ins)).unwrap_or(ctx.len());
                    ctx.insert(pos, CtxEl::Link(l.clone()));
                }
                if track && !own_ins {
                    ctx.push(CtxEl::NewIns(g));
                }
                let tmpl_run = tmpl.and_then(|t| p.items[t].run);
                let content = if a.key.chars().count() == 1 && a.key != "\t" && a.key != "\n" {
                    Content::Char(a.key.chars().next().unwrap())
                } else {
                    Content::NewObj(a.key.clone())
                };
                pl.outs.push(Out {
                    ctx,
                    run: Some(RunKey::New { tmpl: tmpl_run, marks: a.marks, link: a.link.is_some() }),
                    content,
                });
            }
        }
    }
}

/// Comment anchors to weave into a plan, keyed by edited-atom index.
struct CommentMarks {
    marks: Vec<(usize, usize, u32)>,
}

impl CommentMarks {
    /// `n` is the number of atoms in the paragraph being written.
    fn new(comments: &[EComment], n: usize, cx: &mut PatchCtx) -> CommentMarks {
        let mut marks = Vec::new();
        for c in comments {
            let (mut s, mut e) = (c.start, c.end);
            if s == e {
                // A point comment anchors on the character before it, or the
                // first one when it's at the start.
                if n == 0 {
                    cx.warnings.push(format!("comment \"{}\" has no text to attach to; not added", c.text));
                    continue;
                }
                if s == 0 {
                    e = 1;
                } else {
                    s -= 1;
                }
            }
            let id = cx.next_comment_id;
            cx.next_comment_id += 1;
            cx.new_comments.push((id, c.text.clone()));
            marks.push((s, e, id));
        }
        CommentMarks { marks }
    }

    fn before(&self, e: usize, outs: &mut Vec<Out>, p: Option<&Para>) -> bool {
        let mut any = false;
        for (s, _, id) in &self.marks {
            if *s == e {
                let ctx = marker_ctx(outs, p);
                outs.push(Out { ctx, run: None, content: Content::Node(Element::new("w:commentRangeStart").with_attr("w:id", id.to_string()).into()) });
                any = true;
            }
        }
        any
    }

    fn after(&self, e: usize, outs: &mut Vec<Out>, p: Option<&Para>) -> bool {
        let mut any = false;
        for (_, end, id) in &self.marks {
            if *end == e + 1 {
                let ctx = marker_ctx(outs, p);
                for n in comment_end_nodes(*id) {
                    outs.push(Out { ctx: ctx.clone(), run: None, content: Content::Node(n) });
                }
                any = true;
            }
        }
        any
    }
}

/// Context for inserted markup: the previous item's, cut before any deletion.
fn marker_ctx(outs: &[Out], p: Option<&Para>) -> Vec<CtxEl> {
    let Some(last) = outs.last() else { return Vec::new() };
    let cut = last
        .ctx
        .iter()
        .position(|c| match c {
            CtxEl::NewDel(_) => true,
            CtxEl::Orig(w) => p.is_some_and(|p| p.wraps[*w].kind == WrapKind::Del),
            _ => false,
        })
        .unwrap_or(last.ctx.len());
    last.ctx[..cut].to_vec()
}

pub fn comment_end_nodes(id: u32) -> [Node; 2] {
    [
        Element::new("w:commentRangeEnd").with_attr("w:id", id.to_string()).into(),
        Element::new("w:r")
            .with_child(Element::new("w:rPr").with_child(Element::new("w:rStyle").with_attr("w:val", "CommentReference")))
            .with_child(Element::new("w:commentReference").with_attr("w:id", id.to_string()))
            .into(),
    ]
}

/// Re-emit a paragraph unchanged except for a new comment around items [si, ei).
pub fn comment_on_items(p: &Para, si: usize, ei: usize, text: &str, cx: &mut PatchCtx) -> Node {
    let id = cx.next_comment_id;
    cx.next_comment_id += 1;
    cx.new_comments.push((id, text.to_string()));
    let mut pl = Planner { p, outs: Vec::new(), next: 0, last_vis: None, group: 0, last_op: 0, changed: true };
    for i in 0..p.items.len() {
        if i == si {
            let ctx = p.items[i].ctx.iter().map(|w| CtxEl::Orig(*w)).filter(|c| !matches!(c, CtxEl::Orig(w) if p.wraps[*w].kind == WrapKind::Del)).collect();
            pl.outs.push(Out { ctx, run: None, content: Content::Node(Element::new("w:commentRangeStart").with_attr("w:id", id.to_string()).into()) });
        }
        pl.keep(i, None);
        if i + 1 == ei {
            let ctx = marker_ctx(&pl.outs, Some(p));
            for n in comment_end_nodes(id) {
                pl.outs.push(Out { ctx: ctx.clone(), run: None, content: Content::Node(n) });
            }
        }
    }
    let body = build(Some(p), None, pl.outs, cx);
    let mut el = p.shell.clone();
    if let Some(pp) = &p.ppr {
        el.children.push(pp.clone().into());
    }
    el.children.extend(body);
    el.into()
}

// ---------------------------------------------------------------------------
// Building XML from a plan

fn is_code_style(styles: &Styles, id: &str) -> bool {
    let n = styles.name_of(id).unwrap_or(id).to_lowercase();
    n.contains("verbatim") || n.contains("code")
}

pub fn strip_rpr(rpr: &Element) -> Element {
    let mut r = rpr.clone();
    r.remove_children("w:rPrChange");
    r.remove_children("w:ins");
    r.remove_children("w:del");
    r.remove_children("w:moveFrom");
    r.remove_children("w:moveTo");
    r
}

/// Adjust direct formatting so `marks_of(result) == marks` where possible.
pub fn set_marks(rpr: Option<&Element>, marks: Marks, link: bool, styles: &Styles) -> Option<Element> {
    let mut r = rpr.map(strip_rpr).unwrap_or_else(|| Element::new("w:rPr"));
    let style = r.child("w:rStyle").and_then(|s| s.attr("w:val")).map(String::from);
    if let Some(st) = &style {
        let name = styles.name_of(st).unwrap_or(st).to_lowercase();
        let drop = (name == "hyperlink" && !link)
            || (name == "strong" && !marks.bold)
            || (name == "emphasis" && !marks.italic)
            || (is_code_style(styles, st) && !marks.code);
        if drop {
            r.remove_children("w:rStyle");
        }
    }
    if link && r.child("w:rStyle").is_none() {
        if let Some(h) = styles.char_style_named(&["hyperlink"]) {
            r.insert_ordered(Element::new("w:rStyle").with_attr("w:val", h), RPR_ORDER);
        }
    }
    let cur2 = marks_of(Some(&r), styles);
    let toggle = |r: &mut Element, name: &str, cs: Option<&str>, on: bool, cur_on: bool| {
        if on && !cur_on {
            r.insert_ordered(Element::new(name), RPR_ORDER);
        } else if !on {
            r.remove_children(name);
            if let Some(cs) = cs {
                r.remove_children(cs);
            }
        }
    };
    toggle(&mut r, "w:b", Some("w:bCs"), marks.bold, cur2.bold);
    toggle(&mut r, "w:i", Some("w:iCs"), marks.italic, cur2.italic);
    toggle(&mut r, "w:strike", Some("w:dstrike"), marks.strike, cur2.strike);
    let cur3 = marks_of(Some(&r), styles);
    if marks.code && !cur3.code {
        match styles.char_style_named(&["verbatim char", "source code", "code", "html code"]) {
            Some(id) if r.child("w:rStyle").is_none() => {
                r.insert_ordered(Element::new("w:rStyle").with_attr("w:val", id), RPR_ORDER)
            }
            _ => r.insert_ordered(
                Element::new("w:rFonts")
                    .with_attr("w:ascii", "Consolas")
                    .with_attr("w:hAnsi", "Consolas")
                    .with_attr("w:cs", "Consolas"),
                RPR_ORDER,
            ),
        }
    } else if !marks.code && cur3.code {
        r.remove_children("w:rFonts");
    }
    if r.children.is_empty() { None } else { Some(r) }
}

fn deleted_node(n: &Node, del: bool) -> Node {
    // Text boxes inside the node keep their own paragraphs as they are.
    fn walk(x: &mut Element, del: bool) {
        let to = match (x.name.as_str(), del) {
            ("w:t", true) => Some("w:delText"),
            ("w:instrText", true) => Some("w:delInstrText"),
            ("w:delText", false) => Some("w:t"),
            ("w:delInstrText", false) => Some("w:instrText"),
            _ => None,
        };
        if let Some(to) = to {
            x.rename(to);
        }
        if x.is("w:txbxContent") {
            return;
        }
        for c in x.elements_mut() {
            walk(c, del);
        }
    }
    let mut n = n.clone();
    if let Node::Element(e) = &mut n {
        walk(e, del);
    }
    n
}

fn new_obj(key: &str, cx: &mut PatchCtx) -> Option<Node> {
    match key {
        "\t" => Some(Element::new("w:tab").into()),
        "\n" => Some(Element::new("w:br").into()),
        "<!--docex:pagebreak-->" => Some(Element::new("w:br").with_attr("w:type", "page").into()),
        k if k.starts_with("img:") => match new_image(&k[4..], cx) {
            Ok(n) => Some(n),
            Err(e) => {
                cx.warnings.push(format!("image {} skipped: {e}", &k[4..]));
                None
            }
        },
        k if k.starts_with("fn:") => {
            cx.warnings.push(format!("footnote reference [^{}] skipped: an existing footnote can't be referenced twice", &k[3..]));
            None
        }
        k => {
            if k.chars().count() > 1 {
                cx.warnings.push(format!("cannot insert {k}"));
                None
            } else {
                None
            }
        }
    }
}

const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

/// Pixel size of a PNG, JPEG or GIF.
fn image_size(b: &[u8]) -> Option<(u64, u64)> {
    if b.starts_with(b"\x89PNG") && b.len() >= 24 {
        let w = u32::from_be_bytes(b[16..20].try_into().ok()?) as u64;
        let h = u32::from_be_bytes(b[20..24].try_into().ok()?) as u64;
        return Some((w, h));
    }
    if b.starts_with(b"GIF") && b.len() >= 10 {
        return Some((u16::from_le_bytes([b[6], b[7]]) as u64, u16::from_le_bytes([b[8], b[9]]) as u64));
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let m = b[i + 1];
            let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
            if (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC {
                let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u64;
                let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u64;
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}

/// An inline `w:drawing` for a new picture (or one already in the package).
fn new_image(path: &str, cx: &mut PatchCtx) -> anyhow::Result<Node> {
    let existing = cx.rels.iter().find(|r| r.kind.ends_with("/image") && r.target == path).map(|r| r.id.to_string());
    let (rid, size) = match existing {
        Some(id) => (id, None),
        None => {
            let full = match &cx.opts.base_dir {
                Some(b) => b.join(path),
                None => std::path::PathBuf::from(path),
            };
            let data = std::fs::read(&full).map_err(|e| anyhow::anyhow!("{}: {e}", full.display()))?;
            let size = image_size(&data).ok_or_else(|| anyhow::anyhow!("only PNG, JPEG and GIF can be inserted"))?;
            let ext = match &data[..2] {
                [0xFF, 0xD8] => "jpeg",
                [b'G', b'I'] => "gif",
                _ => "png",
            };
            let n = cx.rels.iter().count() + cx.media.len() + 1;
            let name = format!("media/docex{n}.{ext}");
            let id = cx.rels.add(REL_IMAGE, &name, false);
            cx.rels_changed = true;
            cx.media.push((format!("word/{name}"), data));
            (id, Some(size))
        }
    };
    let (w, h) = size.unwrap_or((400, 300));
    // 96 dpi, scaled down to the text width.
    let (mut cx_emu, mut cy_emu) = (w * 9525, h * 9525);
    let max = if cx.max_image_emu > 0 { cx.max_image_emu } else { 5_486_400 };
    if cx_emu > max {
        cy_emu = cy_emu * max / cx_emu;
        cx_emu = max;
    }
    let pid = 10_000 + cx.next_id;
    cx.next_id += 1;
    let name = path.rsplit('/').next().unwrap_or(path);
    let xml = format!(
        r#"<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx_emu}" cy="{cy_emu}"/><wp:docPr id="{pid}" name="Picture {pid}" descr=""/><wp:cNvGraphicFramePr><a:graphicFrameLocks xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" noChangeAspect="1"/></wp:cNvGraphicFramePr><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="0" name="{}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx_emu}" cy="{cy_emu}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing>"#,
        crate::xml::escape_attr(name)
    );
    Ok(crate::xml::Document::parse(&xml)?.root.into())
}

fn note_ref_rpr(styles: &Styles) -> Element {
    match styles.char_style_named(&["footnote reference"]) {
        Some(id) => Element::new("w:rPr").with_child(Element::new("w:rStyle").with_attr("w:val", id)),
        None => Element::new("w:rPr").with_child(Element::new("w:vertAlign").with_attr("w:val", "superscript")),
    }
}

/// A new `w:footnote` with the given paragraphs.
pub fn new_footnote(id: &str, paras: &[EPara], cx: &mut PatchCtx) -> Element {
    let ppr = cx
        .styles
        .by_id
        .values()
        .find(|s| !s.char_style && s.name == "footnote text")
        .map(|s| Element::new("w:pPr").with_child(Element::new("w:pStyle").with_attr("w:val", s.id.clone())));
    let mut note = Element::new("w:footnote").with_attr("w:id", id);
    let empty = empty_cell();
    let paras = if paras.is_empty() { &empty[..] } else { paras };
    for (k, p) in paras.iter().enumerate() {
        let Node::Element(mut el) = new_cell_para(p, ppr.clone(), None, cx) else { continue };
        if k == 0 {
            let pos = el.children.iter().position(|c| !matches!(c, Node::Element(e) if e.is("w:pPr"))).unwrap_or(el.children.len());
            let mark = Element::new("w:r").with_child(note_ref_rpr(cx.styles)).with_child(Element::new("w:footnoteRef"));
            let space = Element::new("w:r").with_child(text_el(" "));
            el.children.insert(pos, space.into());
            el.children.insert(pos, mark.into());
        }
        note.children.push(el.into());
    }
    note
}

/// The runs of a `SEQ` field (auto-numbered caption) showing `shown`.
fn seq_field(label: &str, shown: &str) -> Vec<Node> {
    let run = |c: Element| -> Node { Element::new("w:r").with_child(c).into() };
    let mut instr = Element::new("w:instrText").with_attr("xml:space", "preserve");
    instr.children.push(Node::Text(crate::xml::Text::new(format!(" SEQ {label} \\* ARABIC "))));
    vec![
        run(Element::new("w:fldChar").with_attr("w:fldCharType", "begin")),
        run(instr),
        run(Element::new("w:fldChar").with_attr("w:fldCharType", "separate")),
        run(text_el(shown)),
        run(Element::new("w:fldChar").with_attr("w:fldCharType", "end")),
    ]
}

/// For a new caption like "Figure 3: ...", the atom range of the number and the label.
fn caption_number(atoms: &[EAtom]) -> Option<(usize, usize, String)> {
    let label: String = atoms.iter().map_while(|a| a.key.chars().next().filter(|c| c.is_alphabetic() && a.key.len() == c.len_utf8())).collect();
    let n = label.chars().count();
    if n == 0 || atoms.get(n).map(|a| a.key.as_str()) != Some(" ") {
        return None;
    }
    let digits = atoms[n + 1..].iter().take_while(|a| a.key.len() == 1 && a.key.as_bytes()[0].is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    Some((n + 1, n + 1 + digits, label))
}

struct Frame {
    el: CtxEl,
    node: Element,
}

struct Builder<'a, 'b> {
    p: Option<&'a Para>,
    fallback_rpr: Option<Element>,
    cx: &'a mut PatchCtx<'b>,
    root: Vec<Node>,
    stack: Vec<Frame>,
    run: Option<(RunKey, Element)>,
    text: String,
}

impl Builder<'_, '_> {
    fn in_del(&self) -> bool {
        self.stack.iter().any(|f| match &f.el {
            CtxEl::NewDel(_) => true,
            CtxEl::Orig(w) => self.p.is_some_and(|p| p.wraps[*w].kind == WrapKind::Del),
            _ => false,
        })
    }

    fn container(&mut self) -> &mut Vec<Node> {
        match self.stack.last_mut() {
            Some(f) => &mut f.node.children,
            None => &mut self.root,
        }
    }

    fn flush_text(&mut self) {
        if self.text.is_empty() {
            return;
        }
        let mut t = text_el(&std::mem::take(&mut self.text));
        if self.in_del() {
            t.rename("w:delText");
        }
        if let Some((_, r)) = &mut self.run {
            r.children.push(t.into());
        }
    }

    fn close_run(&mut self) {
        self.flush_text();
        if let Some((_, r)) = self.run.take() {
            self.container().push(r.into());
        }
    }

    fn pop_frame(&mut self) {
        self.close_run();
        let f = self.stack.pop().unwrap();
        let mut node = f.node;
        if let (CtxEl::Orig(w), Some(p)) = (&f.el, self.p) {
            if let Some(cc) = &p.wraps[*w].content_child {
                let mut content = cc.clone();
                content.children = std::mem::take(&mut node.children);
                let mut outer = p.wraps[*w].shell.clone();
                outer.children.push(content.into());
                node = outer;
            }
        }
        self.container().push(node.into());
    }

    fn open_frame(&mut self, el: &CtxEl) {
        self.close_run();
        let node = match el {
            CtxEl::Orig(w) => {
                let p = self.p.unwrap();
                if p.wraps[*w].content_child.is_some() { Element::new("w:sdtContent") } else { p.wraps[*w].shell.clone() }
            }
            CtxEl::NewIns(_) | CtxEl::NewDel(_) => {
                let ins = matches!(el, CtxEl::NewIns(_));
                let name = if ins { "w:ins" } else { "w:del" };
                // Every element gets its own id, also when one edit is split
                // around a hyperlink or field.
                let id = self.cx.id();
                Element::new(name)
                    .with_attr("w:id", id)
                    .with_attr("w:author", self.cx.opts.author.clone())
                    .with_attr("w:date", self.cx.opts.date.clone())
            }
            CtxEl::Link(url) => {
                if let Some(anchor) = url.strip_prefix('#') {
                    Element::new("w:hyperlink").with_attr("w:anchor", anchor).with_attr("w:history", "1")
                } else {
                    let id = self.cx.rels.add(REL_HYPERLINK, url, true);
                    self.cx.rels_changed = true;
                    Element::new("w:hyperlink").with_attr("r:id", id).with_attr("w:history", "1")
                }
            }
        };
        self.stack.push(Frame { el: el.clone(), node });
    }

    fn open_run(&mut self, key: &RunKey) {
        let styles = self.cx.styles;
        let r = match key {
            RunKey::Orig(ri, marks) => {
                let info = &self.p.unwrap().runs[*ri];
                let mut r = info.shell.clone();
                match marks {
                    None => {
                        if let Some(rpr) = &info.rpr {
                            r.children.push(rpr.clone().into());
                        }
                    }
                    Some(m) => {
                        let mut rpr = set_marks(info.rpr.as_ref(), *m, info.rpr.as_ref().is_some_and(|x| {
                            x.child("w:rStyle").and_then(|s| s.attr("w:val")).and_then(|v| styles.name_of(v)) == Some("hyperlink")
                        }), styles)
                        .unwrap_or_else(|| Element::new("w:rPr"));
                        if self.cx.opts.tracked {
                            let mut old = info.rpr.as_ref().map(strip_rpr).unwrap_or_else(|| Element::new("w:rPr"));
                            old.rename("w:rPr");
                            let ch = self.cx.change_el("w:rPrChange").with_child(old);
                            rpr.insert_ordered(ch, RPR_ORDER);
                        }
                        if !rpr.children.is_empty() {
                            r.children.push(rpr.into());
                        }
                    }
                }
                r
            }
            RunKey::New { tmpl, marks, link } => {
                let base = match (tmpl, self.p) {
                    (Some(t), Some(p)) => p.runs[*t].rpr.clone(),
                    _ => self.fallback_rpr.clone(),
                };
                let mut r = Element::new("w:r");
                if let Some(rpr) = set_marks(base.as_ref(), *marks, *link, styles) {
                    r.children.push(rpr.into());
                }
                r
            }
        };
        self.run = Some((key.clone(), r));
    }

    fn push(&mut self, o: Out) {
        let common = self
            .stack
            .iter()
            .zip(&o.ctx)
            .take_while(|(f, c)| f.el == **c)
            .count();
        while self.stack.len() > common {
            self.pop_frame();
        }
        for el in &o.ctx[common..] {
            self.open_frame(el);
        }
        match &o.run {
            None => {
                self.close_run();
                if let Content::Node(n) = o.content {
                    self.container().push(n);
                }
            }
            Some(key) => {
                if self.run.as_ref().is_some_and(|(k, _)| k != key) {
                    self.close_run();
                }
                if self.run.is_none() {
                    self.open_run(key);
                }
                match o.content {
                    Content::Char(c) => self.text.push(c),
                    Content::Node(n) => {
                        self.flush_text();
                        let n = deleted_node(&n, self.in_del());
                        self.run.as_mut().unwrap().1.children.push(n);
                    }
                    Content::NewObj(k) if k.starts_with("math:") || k.starts_with("mathd:") => {
                        // Equations live at paragraph level, not inside a run.
                        self.close_run();
                        let (display, latex) = match k.strip_prefix("mathd:") {
                            Some(l) => (true, l),
                            None => (false, &k[5..]),
                        };
                        let el = crate::math::latex_to_omml(latex, display);
                        self.container().push(el.into());
                        self.cx.math_added = true;
                    }
                    Content::NewObj(k) if k.starts_with("fn:") && self.cx.note_refs.contains_key(&k[3..]) => {
                        // A reference to a new footnote: a superscript run of its own.
                        self.close_run();
                        let label = k[3..].to_string();
                        let id = self.cx.note_refs[&label].clone();
                        self.cx.used_notes.push(label);
                        let run = Element::new("w:r")
                            .with_child(note_ref_rpr(self.cx.styles))
                            .with_child(Element::new("w:footnoteReference").with_attr("w:id", id));
                        self.container().push(run.into());
                    }
                    Content::NewObj(k) => {
                        self.flush_text();
                        if let Some(n) = new_obj(&k, self.cx) {
                            self.run.as_mut().unwrap().1.children.push(n);
                        }
                    }
                }
            }
        }
    }

    fn finish(mut self) -> Vec<Node> {
        while !self.stack.is_empty() {
            self.pop_frame();
        }
        self.close_run();
        self.root
    }
}

fn build(p: Option<&Para>, fallback_rpr: Option<Element>, outs: Vec<Out>, cx: &mut PatchCtx) -> Vec<Node> {
    let mut b = Builder { p, fallback_rpr, cx, root: Vec::new(), stack: Vec::new(), run: None, text: String::new() };
    for o in outs {
        b.push(o);
    }
    b.finish()
}

// ---------------------------------------------------------------------------
// Paragraph properties

fn mark_rpr(ppr: &mut Element) -> &mut Element {
    if ppr.child("w:rPr").is_none() {
        ppr.insert_ordered(Element::new("w:rPr"), PPR_ORDER);
    }
    ppr.child_mut("w:rPr").unwrap()
}

fn mark_inserted_by(ppr: Option<&Element>, author: &str) -> bool {
    ppr.and_then(|p| p.child_path(&["w:rPr", "w:ins"]))
        .is_some_and(|i| i.attr("w:author") == Some(author))
}

fn template_para<'b>(cx: &PatchCtx<'b>, kind: Kind, near: Option<usize>) -> Option<&'b Para> {
    let blocks = cx.blocks;
    let paras = |exact: bool| {
        let near = near.unwrap_or(0);
        let mut best: Option<(&Para, usize)> = None;
        for (i, b) in blocks.iter().enumerate() {
            let Block::Para(p) = b else { continue };
            let ok = if exact {
                p.kind == kind
            } else {
                matches!((p.kind, kind), (Kind::Bullet(_), Kind::Bullet(_)) | (Kind::Number(_), Kind::Number(_)))
            };
            if !ok || (p.is_empty() && kind != Kind::Normal) {
                continue;
            }
            // Prefer the nearest paragraph before the insertion point.
            let dist = if i <= near { (near - i) * 2 } else { (i - near) * 2 + 1 };
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((p, dist));
            }
        }
        best.map(|(p, _)| p)
    };
    paras(true).or_else(|| if kind.is_list() { paras(false) } else { None })
}

fn ensure_numbering(cx: &mut PatchCtx, bullet: bool) -> Option<String> {
    if let Some(id) = cx.styles.num_id_for(bullet) {
        return Some(id);
    }
    let doc = cx.numbering.as_mut()?;
    let root = &mut doc.root;
    let max_abs = root
        .elements()
        .filter(|e| e.is("w:abstractNum"))
        .filter_map(|e| e.attr("w:abstractNumId")?.parse::<u32>().ok())
        .max()
        .map_or(0, |m| m + 1);
    let max_num = root
        .elements()
        .filter(|e| e.is("w:num"))
        .filter_map(|e| e.attr("w:numId")?.parse::<u32>().ok())
        .max()
        .map_or(1, |m| m + 1);
    let mut abs = Element::new("w:abstractNum").with_attr("w:abstractNumId", max_abs.to_string());
    abs.children.push(Element::new("w:multiLevelType").with_attr("w:val", "hybridMultilevel").into());
    for l in 0..9u32 {
        let (fmt, text) = if bullet {
            ("bullet", ["•", "◦", "▪"][l as usize % 3].to_string())
        } else {
            ("decimal", format!("%{}.", l + 1))
        };
        abs.children.push(
            Element::new("w:lvl")
                .with_attr("w:ilvl", l.to_string())
                .with_child(Element::new("w:start").with_attr("w:val", "1"))
                .with_child(Element::new("w:numFmt").with_attr("w:val", fmt))
                .with_child(Element::new("w:lvlText").with_attr("w:val", text))
                .with_child(Element::new("w:lvlJc").with_attr("w:val", "left"))
                .with_child(
                    Element::new("w:pPr").with_child(
                        Element::new("w:ind")
                            .with_attr("w:left", (720 * (l + 1)).to_string())
                            .with_attr("w:hanging", "360"),
                    ),
                )
                .into(),
        );
    }
    let pos = root.children.iter().position(|c| matches!(c, Node::Element(e) if e.is("w:num"))).unwrap_or(root.children.len());
    root.children.insert(pos, abs.into());
    root.children.push(
        Element::new("w:num")
            .with_attr("w:numId", max_num.to_string())
            .with_child(Element::new("w:abstractNumId").with_attr("w:val", max_abs.to_string()))
            .into(),
    );
    cx.numbering_changed = true;
    Some(max_num.to_string())
}

/// pPr (without rPr / sectPr) and base run properties for a new paragraph of `kind`.
fn ppr_for_kind(cx: &mut PatchCtx, kind: Kind, near: Option<usize>) -> (Element, Option<Element>) {
    if let Some(t) = template_para(cx, kind, near) {
        let mut ppr = t.ppr.clone().unwrap_or_else(|| Element::new("w:pPr"));
        ppr.remove_children("w:rPr");
        ppr.remove_children("w:sectPr");
        ppr.remove_children("w:pPrChange");
        if let Kind::Bullet(l) | Kind::Number(l) = kind {
            if let Some(np) = ppr.child_mut("w:numPr") {
                np.insert_ordered(Element::new("w:ilvl").with_attr("w:val", l.to_string()), &["w:ilvl", "w:numId"]);
            } else if l > 0 {
                if let Some(id) = ensure_numbering(cx, matches!(kind, Kind::Bullet(_))) {
                    ppr.insert_ordered(numpr(l, &id), PPR_ORDER);
                }
            }
        }
        let rpr = t
            .visible()
            .find(|(_, i)| t.status(i) != Status::Del && !t.marks(i).code)
            .and_then(|(_, i)| i.run)
            .and_then(|r| t.runs[r].rpr.clone())
            .map(|r| strip_rpr(&r));
        return (ppr, rpr);
    }
    let mut ppr = Element::new("w:pPr");
    let mut rpr = None;
    let style = cx.styles.style_for(kind);
    if let Some(s) = &style {
        ppr.insert_ordered(Element::new("w:pStyle").with_attr("w:val", s.clone()), PPR_ORDER);
    }
    match kind {
        Kind::Bullet(l) | Kind::Number(l) => {
            let style_has_num = style.as_ref().and_then(|s| cx.styles.by_id.get(s)).is_some_and(|s| s.num_id.is_some());
            if !style_has_num || l > 0 {
                if let Some(id) = ensure_numbering(cx, matches!(kind, Kind::Bullet(_))) {
                    ppr.insert_ordered(numpr(l, &id), PPR_ORDER);
                } else {
                    cx.warnings.push("document has no numbering part; list added as plain paragraph".into());
                }
            }
        }
        Kind::Quote if style.is_none() => {
            ppr.insert_ordered(Element::new("w:ind").with_attr("w:left", "720"), PPR_ORDER);
        }
        Kind::Code if style.is_none() => {
            rpr = Some(Element::new("w:rPr").with_child(
                Element::new("w:rFonts").with_attr("w:ascii", "Consolas").with_attr("w:hAnsi", "Consolas"),
            ));
        }
        Kind::Heading(n) if style.is_none() => {
            ppr.insert_ordered(Element::new("w:outlineLvl").with_attr("w:val", (n - 1).to_string()), PPR_ORDER);
            rpr = Some(Element::new("w:rPr").with_child(Element::new("w:b")));
        }
        _ => {}
    }
    (ppr, rpr)
}

fn numpr(lvl: u8, id: &str) -> Element {
    Element::new("w:numPr")
        .with_child(Element::new("w:ilvl").with_attr("w:val", lvl.to_string()))
        .with_child(Element::new("w:numId").with_attr("w:val", id))
}

// ---------------------------------------------------------------------------
// Paragraph-level operations

/// Patch an existing paragraph. Returns the replacement nodes, or `None` if
/// nothing changed.
pub fn patch_para(
    p: &Para,
    orig: &EPara,
    new: Option<&EPara>,
    near: Option<usize>,
    cx: &mut PatchCtx,
) -> Option<Vec<Node>> {
    let comments = new.map(|n| new_comments(orig, n)).unwrap_or_default();
    let (outs, changed) = plan(p, &orig.atoms, new.map(|n| n.atoms.as_slice()), &comments, cx);
    let kind_change = new.map(|n| n.kind).filter(|k| *k != orig.kind && !(orig.kind == Kind::Code && *k == Kind::Code));
    if !changed && kind_change.is_none() && new.is_some() {
        return None;
    }
    let mut ppr = p.ppr.clone();
    if let Some(k) = kind_change {
        cx.stats.kind_changes += 1;
        let (mut np, _) = ppr_for_kind(cx, k, near);
        let old = ppr.clone().unwrap_or_else(|| Element::new("w:pPr"));
        for keep in ["w:rPr", "w:sectPr"] {
            if let Some(e) = old.child(keep) {
                np.insert_ordered(e.clone(), PPR_ORDER);
            }
        }
        if cx.opts.tracked {
            let mut prev = old.clone();
            prev.remove_children("w:rPr");
            prev.remove_children("w:sectPr");
            prev.remove_children("w:pPrChange");
            let ch = cx.change_el("w:pPrChange").with_child(prev);
            np.insert_ordered(ch, PPR_ORDER);
        }
        ppr = Some(np);
    }
    let body = build(Some(p), None, outs, cx);
    if new.is_none() {
        // Whole paragraph deleted: also delete its mark.
        cx.stats.paragraphs_removed += 1;
        let has_sect = p.has_sect_pr();
        let own = mark_inserted_by(p.ppr.as_ref(), &cx.opts.author);
        let only_hidden = !body.iter().any(|n| matches!(n, Node::Element(e) if e.is("w:del") || e.is("w:r") || e.is("w:ins")));
        if !has_sect && ((!cx.opts.tracked) || (own && only_hidden)) {
            // Keep invisible anchors (bookmarks, comment ranges) by moving them nowhere: drop the paragraph.
            return Some(Vec::new());
        }
        if !has_sect && cx.opts.tracked {
            let mut np = ppr.unwrap_or_else(|| Element::new("w:pPr"));
            let del = cx.change_el("w:del");
            mark_rpr(&mut np).insert_ordered(del, RPR_ORDER);
            ppr = Some(np);
        }
    }
    let mut el = p.shell.clone();
    if let Some(pp) = ppr {
        el.children.push(pp.into());
    }
    el.children.extend(body);
    Some(vec![el.into()])
}

/// Comments in the edited paragraph that were not rendered from the document.
fn new_comments(orig: &EPara, new: &EPara) -> Vec<EComment> {
    let existing: Vec<&str> = orig.comments.iter().map(|c| c.text.as_str()).collect();
    new.comments.iter().filter(|c| !c.text.is_empty() && !existing.contains(&c.text.as_str())).cloned().collect()
}

/// A brand-new paragraph.
pub fn new_para(e: &EPara, near: Option<usize>, cx: &mut PatchCtx) -> Node {
    cx.stats.paragraphs_added += 1;
    let (mut ppr, rpr) = ppr_for_kind(cx, e.kind, near);
    let tracked = cx.opts.tracked;
    if tracked {
        let ins = cx.change_el("w:ins");
        mark_rpr(&mut ppr).insert_ordered(ins, RPR_ORDER);
    }
    let cm = CommentMarks::new(&e.comments, e.atoms.len(), cx);
    let mut outs: Vec<Out> = Vec::new();
    // "Figure 3: ..." in a new caption gets a real auto-numbering field.
    let seq = if e.kind == Kind::Caption { caption_number(&e.atoms) } else { None };
    for (k, a) in e.atoms.iter().enumerate() {
        cm.before(k, &mut outs, None);
        if let Some((s, end, label)) = &seq {
            if k == *s {
                let shown: String = e.atoms[*s..*end].iter().map(|a| a.key.as_str()).collect();
                let ctx = if tracked { vec![CtxEl::NewIns(1)] } else { vec![] };
                for n in seq_field(label, &shown) {
                    outs.push(Out { ctx: ctx.clone(), run: None, content: Content::Node(n) });
                }
            }
            if k >= *s && k < *end {
                continue;
            }
        }
        if a.st != Status::Del {
            let mut ctx = Vec::new();
            if let Some(l) = &a.link {
                ctx.push(CtxEl::Link(l.clone()));
            }
            if tracked || a.st == Status::Ins {
                ctx.push(CtxEl::NewIns(1));
            }
            let content = if a.key.chars().count() == 1 && a.key != "\t" && a.key != "\n" {
                Content::Char(a.key.chars().next().unwrap())
            } else {
                Content::NewObj(a.key.clone())
            };
            cx.stats.insertions += 1;
            outs.push(Out { ctx, run: Some(RunKey::New { tmpl: None, marks: a.marks, link: a.link.is_some() }), content });
        }
        cm.after(k, &mut outs, None);
    }
    let body = build(None, rpr, outs, cx);
    let mut p = Element::new("w:p");
    if !ppr.children.is_empty() {
        p.children.push(ppr.into());
    }
    p.children.extend(body);
    p.into()
}

/// Mark everything below `e` as deleted (runs, paragraph marks, table rows).
pub fn mark_deleted(e: &mut Element, cx: &mut PatchCtx) {
    let mut i = 0;
    while i < e.children.len() {
        let Node::Element(c) = &mut e.children[i] else {
            i += 1;
            continue;
        };
        match c.name.as_str() {
            "w:del" | "w:moveFrom" => {}
            "w:r" => {
                let run = deleted_node(&e.children[i], true);
                let del = cx.change_el("w:del").with_child(run);
                e.children[i] = del.into();
            }
            "w:p" => {
                mark_deleted(c, cx);
                let has_sect = c.child_path(&["w:pPr", "w:sectPr"]).is_some();
                if !has_sect {
                    if c.child("w:pPr").is_none() {
                        c.children.insert(0, Element::new("w:pPr").into());
                    }
                    let del = cx.change_el("w:del");
                    let Node::Element(c) = &mut e.children[i] else { unreachable!() };
                    mark_rpr(c.child_mut("w:pPr").unwrap()).insert_ordered(del, RPR_ORDER);
                }
            }
            "w:tr" => {
                mark_deleted(c, cx);
                let del = cx.change_el("w:del");
                let Node::Element(c) = &mut e.children[i] else { unreachable!() };
                if c.child("w:trPr").is_none() {
                    let pos = c.children.iter().position(|n| matches!(n, Node::Element(x) if x.is("w:tc"))).unwrap_or(0);
                    c.children.insert(pos, Element::new("w:trPr").into());
                }
                c.child_mut("w:trPr").unwrap().insert_ordered(del, TRPR_ORDER);
            }
            "w:pPr" | "w:rPr" | "w:sdtPr" | "w:tblPr" | "w:tblGrid" | "w:trPr" | "w:tcPr" => {}
            _ => mark_deleted(c, cx),
        }
        i += 1;
    }
    cx.stats.deletions += 1;
}

// ---------------------------------------------------------------------------
// Tables and other paragraph containers

fn row_key(r: &[Vec<EPara>]) -> String {
    r.iter()
        .map(|c| c.iter().map(|p| atoms_key(&p.atoms)).collect::<Vec<_>>().join("\u{3}"))
        .collect::<Vec<_>>()
        .join("\u{2}")
}

fn row_text(r: &[Vec<EPara>]) -> String {
    r.iter().flat_map(|c| c.iter().map(|p| p.text())).collect::<Vec<_>>().join(" ")
}

fn empty_cell() -> Vec<EPara> {
    vec![EPara { kind: Kind::Normal, atoms: Vec::new(), comments: Vec::new() }]
}

/// Patch the paragraphs of a container (table cell, footnote). The container
/// always keeps at least one paragraph. Returns `None` if unchanged.
pub fn patch_paras(container: &Element, paras: &[Para], orig: &[EPara], new: &[EPara], cx: &mut PatchCtx) -> Option<Element> {
    if orig.len() != paras.len() {
        cx.warnings.push("paragraph layout mismatch in a table cell or note; left unchanged".into());
        return None;
    }
    let ok: Vec<String> = orig.iter().map(|p| atoms_key(&p.atoms)).collect();
    let nk: Vec<String> = new.iter().map(|p| atoms_key(&p.atoms)).collect();
    let o_cm: Vec<usize> = orig.iter().map(|p| p.comments.len()).collect();
    let n_cm: Vec<usize> = new.iter().map(|p| p.comments.len()).collect();
    if ok == nk && o_cm == n_cm {
        return None;
    }
    let ops = align(&ok, &nk, |i, j| similarity(&orig[i].text(), &new[j].text()));
    let at = |p: &Para| *p.path.last().unwrap();
    let mut sp = Splice::default();
    let mut anchor: Option<usize> = None;
    let mut front = Vec::new();
    let (t_ppr, t_rpr) = para_template(paras.first());
    for op in ops {
        match op {
            Align::Keep(i, j) => {
                let p = &paras[i];
                // Same text, maybe a new comment.
                if !new[j].comments.is_empty() {
                    if let Some(r) = patch_para(p, &orig[i], Some(&new[j]), None, cx) {
                        sp.replace.insert(at(p), r);
                    }
                }
                anchor = Some(at(p));
            }
            Align::Modify(i, j) => {
                let p = &paras[i];
                if let Some(r) = patch_para(p, &orig[i], Some(&new[j]), None, cx) {
                    sp.replace.insert(at(p), r);
                }
                anchor = Some(at(p));
            }
            Align::Delete(i) => {
                let p = &paras[i];
                if paras.len() == 1 || new.is_empty() {
                    let empty = EPara { kind: orig[i].kind, atoms: Vec::new(), comments: Vec::new() };
                    if let Some(r) = patch_para(p, &orig[i], Some(&empty), None, cx) {
                        sp.replace.insert(at(p), r);
                    }
                } else if let Some(r) = patch_para(p, &orig[i], None, None, cx) {
                    sp.replace.insert(at(p), r);
                }
                anchor = Some(at(p));
            }
            Align::Insert(j) => {
                let n = new_cell_para(&new[j], t_ppr.clone(), t_rpr.clone(), cx);
                match anchor {
                    Some(a) => sp.after.entry(a).or_default().push(n),
                    None => front.push(n),
                }
            }
        }
    }
    if !front.is_empty() {
        if let Some(p) = paras.first() {
            sp.before.insert(at(p), front);
        }
    }
    if sp.is_empty() {
        return None;
    }
    let mut c = container.clone();
    sp.apply(&mut c.children);
    Some(c)
}

/// Paragraph and run properties to give new paragraphs that look like `p`.
fn para_template(p: Option<&Para>) -> (Option<Element>, Option<Element>) {
    let ppr = p.and_then(|t| t.ppr.clone());
    let rpr = p
        .and_then(|t| t.visible().find(|(_, i)| t.status(i) != Status::Del).and_then(|(_, i)| i.run).map(|r| (t, r)))
        .and_then(|(t, r)| t.runs[r].rpr.clone())
        .map(|r| strip_rpr(&r));
    (ppr, rpr)
}

/// New paragraph inside a table cell or note, with the given template properties.
pub fn new_cell_para(e: &EPara, ppr: Option<Element>, rpr: Option<Element>, cx: &mut PatchCtx) -> Node {
    let mut ppr = ppr.unwrap_or_else(|| Element::new("w:pPr"));
    ppr.remove_children("w:pPrChange");
    ppr.remove_children("w:sectPr");
    if let Some(r) = ppr.child_mut("w:rPr") {
        *r = strip_rpr(r);
    }
    let tracked = cx.opts.tracked;
    if tracked {
        let ins = cx.change_el("w:ins");
        mark_rpr(&mut ppr).insert_ordered(ins, RPR_ORDER);
    }
    let cm = CommentMarks::new(&e.comments, e.atoms.len(), cx);
    let mut outs: Vec<Out> = Vec::new();
    for (k, a) in e.atoms.iter().enumerate() {
        cm.before(k, &mut outs, None);
        if a.st != Status::Del {
            let mut ctx = Vec::new();
            if let Some(l) = &a.link {
                ctx.push(CtxEl::Link(l.clone()));
            }
            if tracked || a.st == Status::Ins {
                ctx.push(CtxEl::NewIns(1));
            }
            cx.stats.insertions += 1;
            outs.push(Out {
                ctx,
                run: Some(RunKey::New { tmpl: None, marks: a.marks, link: a.link.is_some() }),
                content: if a.key.chars().count() == 1 && a.key != "\t" && a.key != "\n" {
                    Content::Char(a.key.chars().next().unwrap())
                } else {
                    Content::NewObj(a.key.clone())
                },
            });
        }
        cm.after(k, &mut outs, None);
    }
    let body = build(None, rpr, outs, cx);
    let mut p = Element::new("w:p");
    if !ppr.children.is_empty() {
        p.children.push(ppr.into());
    }
    p.children.extend(body);
    p.into()
}

fn new_row(tmpl: &Element, t_row: &crate::model::Row, cells: &[Vec<EPara>], cx: &mut PatchCtx) -> Node {
    let mut tr = tmpl.shell();
    let mut ci = 0;
    for c in &tmpl.children {
        match c {
            Node::Element(e) if e.is("w:trPr") => {
                let mut trpr = e.clone();
                trpr.remove_children("w:ins");
                trpr.remove_children("w:del");
                trpr.remove_children("w:trPrChange");
                tr.children.push(trpr.into());
            }
            Node::Element(e) if e.is("w:tc") => {
                let mut tc = e.shell();
                if let Some(pr) = e.child("w:tcPr") {
                    tc.children.push(pr.clone().into());
                }
                let (ppr, rpr) = para_template(t_row.cells.get(ci).and_then(|c| c.paras.first()));
                let empty = empty_cell();
                let paras = cells.get(ci).filter(|c| !c.is_empty()).unwrap_or(&empty);
                for p in paras {
                    tc.children.push(new_cell_para(p, ppr.clone(), rpr.clone(), cx));
                }
                tr.children.push(tc.into());
                ci += 1;
            }
            Node::Element(e) if e.is("w:tblPrEx") => tr.children.push(c.clone()),
            _ => {}
        }
    }
    if cells.len() > ci {
        cx.warnings.push("new table row has more cells than the table; extra cells dropped".into());
    }
    if cx.opts.tracked {
        let ins = cx.change_el("w:ins");
        if tr.child("w:trPr").is_none() {
            let pos = tr.children.iter().position(|n| matches!(n, Node::Element(x) if x.is("w:tc"))).unwrap_or(0);
            tr.children.insert(pos, Element::new("w:trPr").into());
        }
        tr.child_mut("w:trPr").unwrap().insert_ordered(ins, TRPR_ORDER);
    }
    tr.into()
}

pub fn patch_table(
    t: &Table,
    tbl: &Element,
    orig: &[Vec<Vec<EPara>>],
    new: &[Vec<Vec<EPara>>],
    cx: &mut PatchCtx,
) -> Option<Element> {
    if orig.len() != t.rows.len() {
        cx.warnings.push(format!("table b{}: layout mismatch, left unchanged", t.idx));
        return None;
    }
    let ok: Vec<String> = orig.iter().map(|r| row_key(r)).collect();
    let nk: Vec<String> = new.iter().map(|r| row_key(r)).collect();
    let ops = align(&ok, &nk, |i, j| similarity(&row_text(&orig[i]), &row_text(&new[j])));
    let mut sp = Splice::default();
    let mut anchor: Option<usize> = None;
    let mut front = Vec::new();
    for op in ops {
        match op {
            Align::Keep(i, j) if new[j].iter().any(|c| c.iter().any(|p| !p.comments.is_empty())) => {
                patch_row(t, tbl, i, j, orig, new, &mut sp, cx);
                anchor = Some(t.rows[i].child_idx);
            }
            Align::Keep(i, _) => anchor = Some(t.rows[i].child_idx),
            Align::Modify(i, j) => {
                patch_row(t, tbl, i, j, orig, new, &mut sp, cx);
                anchor = Some(t.rows[i].child_idx);
            }
            Align::Delete(i) => {
                let row = &t.rows[i];
                if cx.opts.tracked {
                    let Node::Element(tr) = &tbl.children[row.child_idx] else { continue };
                    let mut wrap = Element::new("x").with_child(tr.clone());
                    mark_deleted(&mut wrap, cx);
                    sp.replace.insert(row.child_idx, wrap.children);
                } else {
                    sp.replace.insert(row.child_idx, Vec::new());
                }
                anchor = Some(row.child_idx);
            }
            Align::Insert(j) => {
                let ti = anchor.and_then(|a| t.rows.iter().position(|r| r.child_idx == a)).unwrap_or(0);
                let Some(trow) = t.rows.get(ti) else { continue };
                let Node::Element(tmpl) = &tbl.children[trow.child_idx] else { continue };
                let n = new_row(tmpl, trow, &new[j], cx);
                match anchor {
                    Some(a) => sp.after.entry(a).or_default().push(n),
                    None => front.push(n),
                }
            }
        }
    }
    if !front.is_empty() {
        if let Some(r) = t.rows.first() {
            sp.before.insert(r.child_idx, front);
        }
    }
    if sp.is_empty() {
        return None;
    }
    let mut out = tbl.clone();
    sp.apply(&mut out.children);
    Some(out)
}

#[allow(clippy::too_many_arguments)]
fn patch_row(t: &Table, tbl: &Element, i: usize, j: usize, orig: &[Vec<Vec<EPara>>], new: &[Vec<Vec<EPara>>], sp: &mut Splice, cx: &mut PatchCtx) {
    let row = &t.rows[i];
    let Node::Element(tr) = &tbl.children[row.child_idx] else { return };
    let mut tr = tr.clone();
    let mut changed = false;
    if new[j].len() > row.cells.len() && new[j][row.cells.len()..].iter().any(|c| c.iter().any(|p| !p.atoms.is_empty())) {
        cx.warnings.push(format!("table b{}: adding columns is not supported; extra cells dropped", t.idx));
    }
    let empty = empty_cell();
    for (ci, cell) in row.cells.iter().enumerate() {
        let nc = new[j].get(ci).unwrap_or(&empty);
        let Node::Element(tc) = &tr.children[row.cell_idx[ci]] else { continue };
        if let Some(ntc) = patch_paras(tc, &cell.paras, &orig[i][ci], nc, cx) {
            tr.children[row.cell_idx[ci]] = ntc.into();
            changed = true;
        }
    }
    if changed {
        sp.replace.insert(row.child_idx, vec![tr.into()]);
    }
}

/// A brand-new table from Markdown rows.
pub fn new_table(rows: &[Vec<Vec<EPara>>], cx: &mut PatchCtx) -> Node {
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    let style = cx.styles.by_id.values().find(|s| s.name == "table grid").map(|s| s.id.clone());
    let mut tblpr = Element::new("w:tblPr");
    match style {
        Some(s) => tblpr.children.push(Element::new("w:tblStyle").with_attr("w:val", s).into()),
        None => {
            let mut b = Element::new("w:tblBorders");
            for side in ["w:top", "w:left", "w:bottom", "w:right", "w:insideH", "w:insideV"] {
                b.children.push(
                    Element::new(side)
                        .with_attr("w:val", "single")
                        .with_attr("w:sz", "4")
                        .with_attr("w:space", "0")
                        .with_attr("w:color", "auto")
                        .into(),
                );
            }
            tblpr.children.push(Element::new("w:tblW").with_attr("w:w", "5000").with_attr("w:type", "pct").into());
            tblpr.children.push(b.into());
        }
    }
    if tblpr.child("w:tblW").is_none() {
        tblpr.children.push(Element::new("w:tblW").with_attr("w:w", "5000").with_attr("w:type", "pct").into());
    }
    let mut tbl = Element::new("w:tbl").with_child(tblpr);
    let mut grid = Element::new("w:tblGrid");
    let w = 9000 / ncols;
    for _ in 0..ncols {
        grid.children.push(Element::new("w:gridCol").with_attr("w:w", w.to_string()).into());
    }
    tbl.children.push(grid.into());
    for (ri, r) in rows.iter().enumerate() {
        let mut tr = Element::new("w:tr");
        if ri == 0 || cx.opts.tracked {
            let mut trpr = Element::new("w:trPr");
            if ri == 0 {
                trpr.children.push(Element::new("w:tblHeader").into());
            }
            if cx.opts.tracked {
                let ins = cx.change_el("w:ins");
                trpr.insert_ordered(ins, TRPR_ORDER);
            }
            tr.children.push(trpr.into());
        }
        for ci in 0..ncols {
            let mut tc = Element::new("w:tc").with_child(
                Element::new("w:tcPr").with_child(Element::new("w:tcW").with_attr("w:w", w.to_string()).with_attr("w:type", "dxa")),
            );
            let empty = vec![EPara { kind: Kind::Normal, atoms: Vec::new(), comments: Vec::new() }];
            let paras = r.get(ci).filter(|c| !c.is_empty()).unwrap_or(&empty);
            for p in paras {
                let mut p = p.clone();
                if ri == 0 {
                    for a in &mut p.atoms {
                        a.marks.bold = true;
                    }
                }
                tc.children.push(new_cell_para(&p, None, None, cx));
            }
            tr.children.push(tc.into());
        }
        tbl.children.push(tr.into());
    }
    tbl.into()
}
