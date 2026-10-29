//! Listing, accepting and rejecting tracked changes.

use crate::xml::{Element, Node};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeKind {
    Insert,
    Delete,
    MoveTo,
    MoveFrom,
    ParagraphInsert,
    ParagraphDelete,
    Format,
    ParagraphFormat,
    RowInsert,
    RowDelete,
    TableFormat,
    SectionFormat,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Change {
    /// 1-based, document order. Stable as long as the document is unchanged.
    pub n: usize,
    pub kind: ChangeKind,
    pub author: String,
    pub date: String,
    pub text: String,
    /// Where the change is: "body", "footnotes" or "endnotes".
    pub part: String,
    /// Word ids (`w:id`) of the elements making up the change.
    pub wids: Vec<String>,
    #[serde(skip)]
    pub scope: usize,
    /// The XML elements making up this change (a paragraph's text and its
    /// mark, a row and its cells, ...).
    #[serde(skip)]
    parts: Vec<Part>,
}

#[derive(Clone, Debug)]
struct Part {
    kind: ChangeKind,
    path: Vec<usize>,
    /// Path of the enclosing paragraph or table row.
    container: Vec<usize>,
}

impl ChangeKind {
    fn polarity(self) -> i8 {
        use ChangeKind::*;
        match self {
            Insert | MoveTo | ParagraphInsert | RowInsert => 1,
            Delete | MoveFrom | ParagraphDelete | RowDelete => -1,
            _ => 0,
        }
    }

    /// The more telling kind when parts are grouped.
    fn rank(self) -> u8 {
        use ChangeKind::*;
        match self {
            RowInsert | RowDelete => 3,
            Insert | Delete | MoveTo | MoveFrom => 2,
            ParagraphInsert | ParagraphDelete => 1,
            _ => 0,
        }
    }
}

/// `grand` is the parent of `parent` (needed for `w:rPr` markers: a paragraph
/// mark inside `w:pPr`, a math run inside `m:r`).
fn kind_of(e: &Element, parent: &Element, grand: Option<&str>) -> Option<ChangeKind> {
    use ChangeKind::*;
    let mark = grand == Some("w:pPr");
    Some(match (e.name.as_str(), parent.name.as_str()) {
        ("w:ins", "w:rPr") if mark => ParagraphInsert,
        ("w:del", "w:rPr") if mark => ParagraphDelete,
        ("w:ins", "w:trPr") => RowInsert,
        ("w:del", "w:trPr") => RowDelete,
        ("w:ins", _) => Insert,
        ("w:del", _) => Delete,
        ("w:moveTo", "w:rPr") if mark => ParagraphInsert,
        ("w:moveFrom", "w:rPr") if mark => ParagraphDelete,
        ("w:moveTo", _) => MoveTo,
        ("w:moveFrom", _) => MoveFrom,
        ("w:rPrChange", _) => Format,
        ("w:pPrChange", _) => ParagraphFormat,
        ("w:tblPrChange" | "w:trPrChange" | "w:tcPrChange" | "w:tblGridChange" | "w:tblPrExChange", _) => TableFormat,
        ("w:sectPrChange", _) => SectionFormat,
        _ => return None,
    })
}

fn excerpt(e: &Element, kind: ChangeKind) -> String {
    let mut s = String::new();
    match kind {
        ChangeKind::ParagraphInsert | ChangeKind::ParagraphDelete => return "¶".into(),
        ChangeKind::Format => {}
        _ => {}
    }
    e.visit(&mut |x| match x.name.as_str() {
        "w:t" | "w:delText" | "m:t" => s.push_str(&x.text_content()),
        "w:tab" => s.push(' '),
        "w:drawing" => s.push_str("[image]"),
        _ => {}
    });
    if s.chars().count() > 80 {
        s = s.chars().take(77).collect::<String>() + "...";
    }
    s
}

/// All tracked changes below `root`, in document order. Adjacent parts of one
/// edit (same author, date and direction, same paragraph or row) form one change.
pub fn list(root: &Element) -> Vec<Change> {
    let mut raw = Vec::new();
    let mut path = Vec::new();
    let mut names = Vec::new();
    let mut anc = Vec::new();
    collect(root, &mut path, &mut names, &mut anc, &mut raw);
    let mut out: Vec<Change> = Vec::new();
    for ch in raw {
        if let Some(last) = out.last_mut() {
            let root = &last.parts[0];
            let lp = last.parts.last().unwrap();
            let p = &ch.parts[0];
            let related = p.container.starts_with(&root.container) || root.container.starts_with(&p.container);
            // Separate text edits stay separate; a paragraph mark or row change
            // pulls in the text it belongs to.
            let structural = |k: ChangeKind| k.rank() != 2;
            let joins = structural(p.kind) || last.parts.iter().any(|x| structural(x.kind));
            if related
                && joins
                && last.author == ch.author
                && last.date == ch.date
                && lp.kind.polarity() == p.kind.polarity()
                && p.kind.polarity() != 0
            {
                if ch.kind.rank() > last.kind.rank() {
                    last.kind = ch.kind;
                }
                if !ch.text.is_empty() && ch.text != "¶" {
                    if last.text == "¶" {
                        last.text.clear();
                    }
                    last.text.push_str(&ch.text);
                }
                last.parts.extend(ch.parts);
                last.wids.extend(ch.wids);
                continue;
            }
        }
        let mut ch = ch;
        ch.n = out.len() + 1;
        out.push(ch);
    }
    out
}

fn collect<'a>(e: &'a Element, path: &mut Vec<usize>, names: &mut Vec<String>, anc: &mut Vec<&'a Element>, out: &mut Vec<Change>) {
    let grand = anc.last().copied();
    for (i, c) in e.children.iter().enumerate() {
        let Node::Element(c) = c else { continue };
        path.push(i);
        if let Some(kind) = kind_of(c, e, grand.map(|g| g.name.as_str())) {
            // A marker in a math run's properties changes the whole run.
            let text = match (kind, grand) {
                (ChangeKind::Format, _) => String::new(),
                (ChangeKind::Insert | ChangeKind::Delete, Some(g)) if e.is("w:rPr") => excerpt(g, ChangeKind::Insert),
                _ => excerpt(c, kind),
            };
            // names[k] is the name of the element at path[..k] (k >= 1).
            let container = (1..path.len())
                .rev()
                .find(|k| matches!(names.get(*k - 1).map(String::as_str), Some("w:p") | Some("w:tr")))
                .map(|k| path[..k].to_vec())
                .unwrap_or_else(|| path[..path.len() - 1].to_vec());
            out.push(Change {
                n: 0,
                part: "body".into(),
                wids: c.attr("w:id").map(|w| vec![w.to_string()]).unwrap_or_default(),
                scope: 0,
                kind,
                author: c.attr("w:author").unwrap_or("").to_string(),
                date: c.attr("w:date").unwrap_or("").to_string(),
                text,
                parts: vec![Part { kind, path: path.clone(), container }],
            });
        }
        names.push(c.name.clone());
        // Text box content exists twice (modern + VML fallback); list it once.
        if !c.is("mc:Fallback") {
            anc.push(e);
            collect(c, path, names, anc, out);
            anc.pop();
        }
        names.pop();
        path.pop();
    }
}

/// Fill in the run text for format changes (needs the run, two levels up).
pub fn annotate(root: &Element, changes: &mut [Change]) {
    for ch in changes.iter_mut().filter(|c| c.kind == ChangeKind::Format) {
        let path = &ch.parts[0].path;
        if path.len() >= 2 {
            if let Some(run) = at(root, &path[..path.len() - 2]) {
                ch.text = excerpt(run, ChangeKind::Insert);
            }
        }
    }
}

fn at<'a>(root: &'a Element, path: &[usize]) -> Option<&'a Element> {
    let mut cur = root;
    for i in path {
        cur = cur.children.get(*i)?.as_element()?;
    }
    Some(cur)
}

fn at_mut<'a>(root: &'a mut Element, path: &[usize]) -> Option<&'a mut Element> {
    let mut cur = root;
    for i in path {
        cur = cur.children.get_mut(*i)?.as_element_mut()?;
    }
    Some(cur)
}

fn undelete(nodes: &mut [Node]) {
    for n in nodes {
        if let Node::Element(e) = n {
            e.visit_mut(&mut |x| match x.name.as_str() {
                "w:delText" => x.rename("w:t"),
                "w:delInstrText" => x.rename("w:instrText"),
                _ => {}
            });
        }
    }
}

/// Replace the element at `path` by its children.
fn unwrap_at(root: &mut Element, path: &[usize], undel: bool) {
    let (last, parent) = path.split_last().unwrap();
    let Some(p) = at_mut(root, parent) else { return };
    let Node::Element(e) = p.children.remove(*last) else { return };
    let mut kids = e.children;
    if undel {
        undelete(&mut kids);
    }
    for (k, n) in kids.into_iter().enumerate() {
        p.children.insert(last + k, n);
    }
}

fn remove_at(root: &mut Element, path: &[usize]) {
    let (last, parent) = path.split_last().unwrap();
    if let Some(p) = at_mut(root, parent) {
        p.children.remove(*last);
    }
}

fn has_content(p: &Element) -> bool {
    p.elements().any(|e| !matches!(e.name.as_str(), "w:pPr" | "w:bookmarkStart" | "w:bookmarkEnd" | "w:proofErr"))
}

/// Remove a paragraph mark: join paragraph `path` with the next paragraph.
fn merge_with_next(root: &mut Element, ppath: &[usize]) {
    let (last, parent) = ppath.split_last().unwrap();
    let Some(container) = at_mut(root, parent) else { return };
    let Some(Node::Element(p)) = container.children.get(*last) else { return };
    if !has_content(p) {
        // Keep invisible anchors by moving them into the next paragraph.
        let anchors: Vec<Node> = p.children.iter().filter(|c| !matches!(c, Node::Element(e) if e.is("w:pPr"))).cloned().collect();
        container.children.remove(*last);
        if let Some(Node::Element(next)) = container.children.get_mut(*last) {
            if next.is("w:p") {
                let pos = next.children.iter().position(|c| !matches!(c, Node::Element(e) if e.is("w:pPr"))).unwrap_or(next.children.len());
                for (k, a) in anchors.into_iter().enumerate() {
                    next.children.insert(pos + k, a);
                }
            }
        }
        return;
    }
    let next_is_p = matches!(container.children.get(last + 1), Some(Node::Element(e)) if e.is("w:p"));
    if !next_is_p {
        return;
    }
    let Node::Element(p) = container.children.remove(*last) else { return };
    let content: Vec<Node> = p.children.into_iter().filter(|c| !matches!(c, Node::Element(e) if e.is("w:pPr"))).collect();
    if let Some(Node::Element(next)) = container.children.get_mut(*last) {
        let pos = next.children.iter().position(|c| !matches!(c, Node::Element(e) if e.is("w:pPr"))).unwrap_or(next.children.len());
        for (k, n) in content.into_iter().enumerate() {
            next.children.insert(pos + k, n);
        }
    }
}

/// Path of the first element named `name` with this `w:id`.
fn find_by_id(e: &Element, name: &str, id: &str, path: &mut Vec<usize>) -> bool {
    for (i, c) in e.children.iter().enumerate() {
        let Node::Element(c) = c else { continue };
        path.push(i);
        if c.name == name && c.attr("w:id") == Some(id) {
            return true;
        }
        if !c.is("mc:Fallback") && find_by_id(c, name, id, path) {
            return true;
        }
        path.pop();
    }
    false
}

/// Remove the row at `row` and, if it was the table's last row, the table.
fn remove_row(root: &mut Element, row: &[usize]) {
    remove_at(root, row);
    let tbl = &row[..row.len() - 1];
    if let Some(t) = at(root, tbl) {
        if t.is("w:tbl") && !t.elements().any(|e| e.is("w:tr")) {
            remove_at(root, tbl);
        }
    }
}

/// Accept (`accept = true`) or reject the selected changes.
pub fn resolve(root: &mut Element, changes: &[Change], accept: bool) -> usize {
    use ChangeKind::*;
    let parts: Vec<&Part> = changes.iter().flat_map(|c| c.parts.iter()).collect();
    // Formatting records are found again by id at the end: resolving them
    // rewrites property elements, which would invalidate other parts' paths.
    let mut formats: Vec<(String, String, ChangeKind)> = Vec::new();
    let mut sel: Vec<&Part> = Vec::new();
    for p in parts {
        if matches!(p.kind, Format | ParagraphFormat | TableFormat | SectionFormat) {
            if let Some(e) = at(root, &p.path) {
                formats.push((e.name.clone(), e.attr("w:id").unwrap_or("").to_string(), p.kind));
            }
        } else {
            sel.push(p);
        }
    }
    // Deepest / latest first, so earlier paths stay valid.
    sel.sort_by(|a, b| b.path.cmp(&a.path));
    for ch in sel {
        let path = &ch.path;
        let keep_content = matches!((ch.kind, accept), (Insert | MoveTo, true) | (Delete | MoveFrom, false));
        match ch.kind {
            Insert | Delete | MoveTo | MoveFrom => {
                let in_rpr = path.len() >= 2 && at(root, &path[..path.len() - 1]).is_some_and(|p| p.is("w:rPr"));
                if in_rpr {
                    // A marker in a (math) run's properties: the run is the content.
                    let rpr = &path[..path.len() - 1];
                    let run = &path[..path.len() - 2];
                    if keep_content {
                        remove_at(root, path);
                        if at(root, rpr).is_some_and(|r| r.children.iter().all(|c| !matches!(c, Node::Element(_)))) {
                            remove_at(root, rpr);
                        }
                    } else {
                        remove_at(root, run);
                    }
                } else if keep_content {
                    unwrap_at(root, path, !accept);
                } else {
                    remove_at(root, path);
                }
            }
            ParagraphInsert | ParagraphDelete => {
                // path = [... p, pPr, rPr, ins|del]
                let mark_stays = (ch.kind == ParagraphInsert) == accept;
                remove_at(root, path);
                if let Some(rpr) = at_mut(root, &path[..path.len() - 1]) {
                    if rpr.children.is_empty() {
                        remove_at(root, &path[..path.len() - 1]);
                    }
                }
                if !mark_stays && path.len() >= 3 {
                    merge_with_next(root, &path[..path.len() - 3]);
                }
            }
            RowInsert | RowDelete => {
                // path = [... tr, trPr, ins|del]
                let row_stays = (ch.kind == RowInsert) == accept;
                if row_stays {
                    remove_at(root, path);
                } else if path.len() >= 2 {
                    remove_row(root, &path[..path.len() - 2]);
                }
            }
            Format | ParagraphFormat | TableFormat | SectionFormat => {}
        }
    }
    for (name, id, kind) in formats {
        let mut path = Vec::new();
        if !find_by_id(root, &name, &id, &mut path) {
            continue;
        }
        if accept || kind == SectionFormat {
            remove_at(root, &path);
            continue;
        }
        let (last, parent) = path.split_last().unwrap();
        let Some(owner) = at_mut(root, parent) else { continue };
        let Node::Element(chg) = owner.children.remove(*last) else { continue };
        let old: Vec<Node> = chg.elements().next().map(|e| e.children.clone()).unwrap_or_default();
        // Kept outside the change record: the paragraph mark's properties and
        // section properties (pPr), and tracked-change markers (rPr).
        let keep: Vec<Node> = owner
            .children
            .iter()
            .filter(|c| match c {
                Node::Element(e) if kind == ParagraphFormat => e.is("w:rPr") || e.is("w:sectPr"),
                Node::Element(e) => matches!(e.name.as_str(), "w:ins" | "w:del" | "w:moveFrom" | "w:moveTo"),
                _ => false,
            })
            .cloned()
            .collect();
        if kind == ParagraphFormat {
            owner.children = old;
            owner.children.extend(keep);
        } else {
            // Markers come first in w:rPr.
            owner.children = keep;
            owner.children.extend(old);
        }
    }
    changes.len()
}

/// The text and change structure of a text box, ignoring formatting.
fn box_signature(e: &Element) -> String {
    let mut s = String::new();
    e.visit(&mut |x| match x.name.as_str() {
        "w:p" | "w:ins" | "w:del" | "w:moveFrom" | "w:moveTo" => {
            s.push('<');
            s.push_str(&x.name);
        }
        "w:t" | "w:delText" => {
            s.push(if x.is("w:t") { '|' } else { '~' });
            s.push_str(&x.text_content());
        }
        _ => {}
    });
    s
}

/// Make the VML fallback copy of each text box match its modern copy, where
/// the two now differ in text or tracked changes (copies that differ only in
/// formatting are left untouched).
pub fn sync_fallbacks(root: &mut Element) {
    root.visit_mut(&mut |e| {
        if !e.is("mc:AlternateContent") {
            return;
        }
        let mut modern = Vec::new();
        if let Some(choice) = e.child("mc:Choice") {
            choice.descendants("w:txbxContent", &mut modern);
        }
        let modern: Vec<Element> = modern.into_iter().cloned().collect();
        if modern.is_empty() {
            return;
        }
        if let Some(fb) = e.child_mut("mc:Fallback") {
            let mut k = 0;
            fb.visit_mut(&mut |x| {
                if x.is("w:txbxContent") {
                    if let Some(m) = modern.get(k) {
                        if box_signature(m) != box_signature(x) {
                            x.children = m.children.clone();
                        }
                    }
                    k += 1;
                }
            });
        }
    });
}
