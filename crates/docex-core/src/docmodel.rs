//! A JSON document model for graphical editors.
//!
//! It is produced from the same rendered-and-reparsed blocks the Markdown
//! path uses, so sending an unchanged model back is a no-op. Spans carry the
//! author, date and Word id of tracked changes and the ids of comments, for
//! display and for accepting / rejecting.

use crate::mdparse::{EAtom, EBlock, ECell, EComment, EPara, ETable, Parsed};
use crate::model::{Block, Marks, Para, Status, WrapKind, What};
use crate::styles::Kind;
use crate::xml::Node;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_plain(s: &Status) -> bool {
    *s == Status::Plain
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Span {
    /// Text (empty for objects).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub t: String,
    /// An object instead of text: `img:media/x.png`, `fn:3`, `\t`, `\n`,
    /// `<!--docex:shape-->`, ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obj: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub b: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub i: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub s: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub c: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "is_plain")]
    pub st: Status,
    /// Tracked change details (display only; ignored on import).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wid: Option<String>,
    /// Comments anchored on this text (display only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NewComment {
    /// Character offsets into the paragraph's text (objects count as one).
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MPara {
    /// normal, h1..h6, bullet, number, quote, code, caption
    pub kind: String,
    #[serde(default)]
    pub level: u8,
    pub spans: Vec<Span>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new_comments: Vec<NewComment>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MBlock {
    Para(MPara),
    /// Rows of visible cells (a cell spanning rows is listed in its first
    /// row only), each a list of paragraphs.
    Table {
        rows: Vec<Vec<Vec<MPara>>>,
        /// `[colspan, rowspan]` of each cell, row by row; empty when no cell is merged.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        spans: Vec<Vec<[usize; 2]>>,
    },
    /// Content docex does not edit (TOC, content control, complex table).
    Opaque {
        id: usize,
        #[serde(default)]
        label: String,
        #[serde(default)]
        preview: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MBox {
    pub id: String,
    /// Index of the block the box follows; `None` = before the first block.
    pub after: Option<usize>,
    pub paras: Vec<MPara>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MNote {
    pub label: String,
    pub paras: Vec<MPara>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DocModel {
    pub blocks: Vec<MBlock>,
    #[serde(default)]
    pub boxes: Vec<MBox>,
    #[serde(default)]
    pub notes: Vec<MNote>,
}

pub fn kind_name(k: Kind) -> (String, u8) {
    match k {
        Kind::Normal => ("normal".into(), 0),
        Kind::Heading(n) => (format!("h{n}"), 0),
        Kind::Bullet(l) => ("bullet".into(), l),
        Kind::Number(l) => ("number".into(), l),
        Kind::Quote => ("quote".into(), 0),
        Kind::Code => ("code".into(), 0),
        Kind::Caption => ("caption".into(), 0),
    }
}

pub fn kind_of(name: &str, level: u8) -> Kind {
    match name {
        "bullet" => Kind::Bullet(level.min(8)),
        "number" => Kind::Number(level.min(8)),
        "quote" => Kind::Quote,
        "code" => Kind::Code,
        "caption" => Kind::Caption,
        h if h.len() == 2 && h.starts_with('h') => Kind::Heading(h[1..].parse::<u8>().unwrap_or(1).clamp(1, 6)),
        _ => Kind::Normal,
    }
}

/// Change details and comment ids for each atom of a rendered paragraph.
type Extra = (Option<usize>, Option<(String, String, String)>, Vec<String>);

fn enrich(p: &Para, e: &EPara) -> Vec<Extra> {
    let map = crate::patch::map_atoms(p, &e.atoms);
    // Comment ids open at each item.
    let mut open: Vec<String> = Vec::new();
    let mut at_item: Vec<Vec<String>> = Vec::with_capacity(p.items.len());
    for it in &p.items {
        if let What::Hidden(Node::Element(el)) = &it.what {
            let id = el.attr("w:id").unwrap_or("").to_string();
            match el.name.as_str() {
                "w:commentRangeStart" => open.push(id),
                "w:commentRangeEnd" => open.retain(|o| *o != id),
                _ => {}
            }
        }
        at_item.push(open.clone());
    }
    map.iter()
        .map(|m| {
            let Some(ii) = m else { return (None, None, Vec::new()) };
            let it = &p.items[*ii];
            let change = it.ctx.iter().rev().map(|w| &p.wraps[*w]).find(|w| matches!(w.kind, WrapKind::Ins | WrapKind::Del)).map(|w| {
                (
                    w.author.clone().unwrap_or_default(),
                    w.shell.attr("w:date").unwrap_or("").to_string(),
                    w.shell.attr("w:id").unwrap_or("").to_string(),
                )
            });
            (Some(*ii), change, at_item[*ii].clone())
        })
        .collect()
}

fn para_to_model(p: Option<&Para>, e: &EPara, known_comments: &HashSet<String>) -> MPara {
    let extra = p.map(|p| enrich(p, e)).unwrap_or_default();
    let (kind, level) = kind_name(e.kind);
    let mut spans: Vec<Span> = Vec::new();
    for (k, a) in e.atoms.iter().enumerate() {
        let (item, change, comments) = extra.get(k).cloned().unwrap_or((None, None, Vec::new()));
        let comments: Vec<String> = comments.into_iter().filter(|c| known_comments.contains(c)).collect();
        let (author, date, wid) = match (a.st, change) {
            (Status::Plain, _) | (_, None) => (None, None, None),
            (_, Some((au, d, w))) => (Some(au), Some(d), Some(w)),
        };
        let mut s = Span {
            b: a.marks.bold,
            i: a.marks.italic,
            s: a.marks.strike,
            c: a.marks.code,
            link: a.link.clone(),
            st: a.st,
            author,
            date,
            wid,
            comments,
            ..Default::default()
        };
        let is_obj = a.key.chars().count() != 1 || a.key == "\t" || a.key == "\n";
        if is_obj {
            s.obj = Some(a.key.clone());
            if let (true, Some(p), Some(ii)) = (a.key.starts_with("img:"), p, item) {
                if let What::Obj { node, .. } = &p.items[ii].what {
                    s.alt = Some(crate::model::image_alt(node)).filter(|a| !a.is_empty());
                }
            }
            spans.push(s);
            continue;
        }
        if let Some(last) = spans.last_mut() {
            if last.obj.is_none() && (Span { t: String::new(), ..last.clone() }) == s {
                last.t.push_str(&a.key);
                continue;
            }
        }
        s.t = a.key.clone();
        spans.push(s);
    }
    MPara { kind, level, spans, new_comments: Vec::new() }
}

fn spans_of(t: &ETable) -> Vec<Vec<[usize; 2]>> {
    if !t.iter().flatten().any(|c| c.colspan != 1 || c.rowspan != 1) {
        return Vec::new();
    }
    t.iter().map(|r| r.iter().map(|c| [c.colspan, c.rowspan]).collect()).collect()
}

/// A model for Markdown text (e.g. pasted), not tied to a document.
pub fn from_markdown(md: &str) -> DocModel {
    let parsed = crate::mdparse::parse_full(md);
    let none = HashSet::new();
    let mut out = DocModel::default();
    for b in &parsed.blocks {
        out.blocks.push(match b {
            EBlock::Para(e) => MBlock::Para(para_to_model(None, e, &none)),
            EBlock::Table(rows) => MBlock::Table {
                rows: rows.iter().map(|r| r.iter().map(|c| c.paras.iter().map(|e| para_to_model(None, e, &none)).collect()).collect()).collect(),
                spans: spans_of(rows),
            },
            EBlock::Opaque(_) => continue,
        });
    }
    out
}

/// Build the model from blocks rendered and reparsed (`parsed`), the
/// projection they came from, and the render maps.
pub fn export(
    blocks: &[Block],
    rendered: &crate::render::Rendered,
    parsed: &Parsed,
    notes: &[crate::model::Note],
    boxes: &[crate::model::TextBox],
    known_comments: &HashSet<String>,
) -> DocModel {
    let mut out = DocModel::default();
    for (i, eb) in parsed.blocks.iter().enumerate() {
        let b = rendered.map.get(i).and_then(|bi| blocks.get(*bi));
        let mb = match (eb, b) {
            (EBlock::Para(e), Some(Block::Para(p))) => MBlock::Para(para_to_model(Some(p), e, known_comments)),
            (EBlock::Para(e), _) => MBlock::Para(para_to_model(None, e, known_comments)),
            (EBlock::Table(rows), Some(Block::Table(t))) => {
                // Visible cells map to the Word cells where their merges start.
                let anchors = t.anchors();
                MBlock::Table {
                    rows: rows
                        .iter()
                        .enumerate()
                        .map(|(ri, r)| {
                            r.iter()
                                .enumerate()
                                .map(|(ci, c)| {
                                    let cell = anchors.get(ri).and_then(|a| a.get(ci)).and_then(|a| t.rows[ri].cells.get(a.cell));
                                    c.paras
                                        .iter()
                                        .enumerate()
                                        .map(|(pi, e)| para_to_model(cell.and_then(|c| c.paras.get(pi)), e, known_comments))
                                        .collect()
                                })
                                .collect()
                        })
                        .collect(),
                    spans: spans_of(rows),
                }
            }
            (EBlock::Table(rows), _) => MBlock::Table {
                rows: rows.iter().map(|r| r.iter().map(|c| c.paras.iter().map(|e| para_to_model(None, e, known_comments)).collect()).collect()).collect(),
                spans: spans_of(rows),
            },
            (EBlock::Opaque(n), b) => {
                let (label, preview) = match b {
                    Some(Block::Opaque { label, .. }) => (label.clone(), String::new()),
                    Some(Block::Table(t)) => (
                        "table docex can't edit (read-only)".into(),
                        t.rows
                            .iter()
                            .map(|r| {
                                r.cells
                                    .iter()
                                    .map(|c| c.paras.iter().map(|p| crate::render::view_text(p, crate::render::View::Accepted)).collect::<Vec<_>>().join(" "))
                                    .collect::<Vec<_>>()
                                    .join(" | ")
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                    _ => (String::new(), String::new()),
                };
                MBlock::Opaque { id: *n, label, preview }
            }
        };
        out.blocks.push(mb);
    }
    for (k, used) in &rendered.boxes {
        let b = &boxes[*k];
        let Some((_, eparas)) = parsed.boxes.iter().find(|x| x.0 == b.id) else { continue };
        let after = rendered
            .map
            .iter()
            .rposition(|bi| blocks.get(*bi).is_some_and(|bl| bl.body_idx() <= b.anchor));
        let paras = eparas
            .iter()
            .enumerate()
            .map(|(pi, e)| para_to_model(used.get(pi).and_then(|i| b.paras.get(*i)), e, known_comments))
            .collect();
        out.boxes.push(MBox { id: b.id.clone(), after, paras });
    }
    for (label, used) in &rendered.notes {
        let Some(n) = notes.iter().find(|n| &n.label == label) else { continue };
        let Some((_, eparas)) = parsed.notes.iter().find(|x| &x.0 == label) else { continue };
        let paras = eparas
            .iter()
            .enumerate()
            .map(|(pi, e)| para_to_model(used.get(pi).and_then(|i| n.paras.get(*i)), e, known_comments))
            .collect();
        out.notes.push(MNote { label: label.clone(), paras });
    }
    out
}

fn para_from_model(m: &MPara) -> EPara {
    let mut atoms = Vec::new();
    for s in &m.spans {
        let marks = Marks { bold: s.b, italic: s.i, strike: s.s, code: s.c };
        let mk = |key: String| EAtom { key, marks, link: s.link.clone().filter(|l| !l.is_empty()), st: s.st };
        match &s.obj {
            Some(o) => atoms.push(mk(o.clone())),
            None => {
                for ch in s.t.chars() {
                    atoms.push(mk(ch.to_string()));
                }
            }
        }
    }
    // Same trimming as the Markdown parser.
    let ws = |a: &EAtom| a.key.chars().all(char::is_whitespace);
    let lead = atoms.iter().take_while(|a| ws(a)).count();
    atoms.drain(..lead);
    while atoms.last().is_some_and(ws) {
        atoms.pop();
    }
    let n = atoms.len();
    let comments = m
        .new_comments
        .iter()
        .map(|c| EComment {
            start: c.start.saturating_sub(lead).min(n),
            end: c.end.saturating_sub(lead).min(n),
            text: c.text.clone(),
        })
        .collect();
    EPara { kind: kind_of(&m.kind, m.level), atoms, comments }
}

/// The model as parsed blocks, ready for `Docx::apply_parsed`.
pub fn import(m: &DocModel) -> Parsed {
    let mut blocks = Vec::new();
    for b in &m.blocks {
        match b {
            MBlock::Para(p) => {
                let e = para_from_model(p);
                if !e.atoms.is_empty() || !e.comments.is_empty() {
                    blocks.push(EBlock::Para(e));
                }
            }
            MBlock::Table { rows, spans } => blocks.push(EBlock::Table(
                rows.iter()
                    .enumerate()
                    .map(|(ri, r)| {
                        r.iter()
                            .enumerate()
                            .map(|(ci, c)| {
                                let v: Vec<EPara> = c.iter().map(para_from_model).collect();
                                let paras = if v.is_empty() { vec![EPara { kind: Kind::Normal, atoms: Vec::new(), comments: Vec::new() }] } else { v };
                                let [colspan, rowspan] = spans.get(ri).and_then(|s| s.get(ci)).copied().unwrap_or([1, 1]);
                                ECell { paras, colspan: colspan.max(1), rowspan: rowspan.max(1) }
                            })
                            .collect()
                    })
                    .collect(),
            )),
            MBlock::Opaque { id, .. } => blocks.push(EBlock::Opaque(*id)),
        }
    }
    let paras = |v: &[MPara]| v.iter().map(para_from_model).filter(|e| !e.atoms.is_empty() || !e.comments.is_empty()).collect();
    Parsed {
        blocks,
        notes: m.notes.iter().map(|n| (n.label.clone(), paras(&n.paras))).collect(),
        boxes: m.boxes.iter().map(|b| (b.id.clone(), paras(&b.paras))).collect(),
    }
}
