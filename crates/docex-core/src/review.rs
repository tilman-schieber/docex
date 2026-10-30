//! A review list for people and agents: every tracked change and comment
//! thread with a stable id, its section, surrounding text, and old -> new.
//!
//! Ids come from Word's own change ids (`c12`; `f12`/`e12` in footnotes and
//! endnotes) and comment ids (`m3`), so they do not shift when other
//! changes are accepted or rejected.

use crate::comments::Comment;
use crate::docmodel::{DocModel, MBlock, MPara};
use crate::model::Status;
use crate::revisions::{Change, ChangeKind};
use serde::Serialize;
use std::collections::HashMap;

const CONTEXT: usize = 40;

#[derive(Clone, Debug, Serialize)]
pub struct Reply {
    pub id: String,
    pub author: String,
    pub date: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Item {
    /// Stable id: `c12` (body), `f12` / `e12` (foot/endnotes), `m3` (comment).
    pub id: String,
    /// insert, delete, replace, format, comment
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub author: String,
    pub date: String,
    /// Heading path, outermost first.
    pub section: Vec<String>,
    pub part: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub before: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub after: String,
    /// Comment text, or a description for changes without visible text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<Reply>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resolved: bool,
    /// Word change ids making up the item.
    #[serde(skip)]
    pub wids: Vec<String>,
    #[serde(skip)]
    order: (usize, usize),
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Person {
    pub name: String,
    pub inserts: usize,
    pub deletes: usize,
    pub replaces: usize,
    pub other: usize,
    pub comments: usize,
    pub replies: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Summary {
    pub changes: usize,
    pub threads: usize,
    pub open_threads: usize,
    pub people: Vec<Person>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Review {
    pub summary: Summary,
    pub items: Vec<Item>,
}

/// One character of a paragraph with its change and comment info.
struct Ch {
    s: String,
    st: Status,
    wid: Option<String>,
    comments: Vec<String>,
}

struct Range {
    para: usize,
    start: usize,
    end: usize,
    st: Status,
    text: String,
}

struct ParaInfo {
    chars: Vec<Ch>,
    section: Vec<String>,
}

fn obj_text(o: &str) -> String {
    if let Some(l) = o.strip_prefix("mathd:") {
        format!("$${l}$$")
    } else if let Some(l) = o.strip_prefix("math:") {
        format!("${l}$")
    } else if let Some(n) = o.strip_prefix("fn:") {
        format!("[^{n}]")
    } else if o.starts_with("img:") {
        "[image]".into()
    } else if o == "\n" || o == "\t" {
        " ".into()
    } else {
        format!("[{}]", o.trim_start_matches("<!--docex:").trim_end_matches("-->"))
    }
}

fn para_chars(p: &MPara) -> Vec<Ch> {
    let mut out = Vec::new();
    for s in &p.spans {
        let mk = |text: String| Ch { s: text, st: s.st, wid: s.wid.clone(), comments: s.comments.clone() };
        match &s.obj {
            Some(o) => out.push(mk(obj_text(o))),
            None => out.extend(s.t.chars().map(|c| mk(c.to_string()))),
        }
    }
    out
}

fn visible(chars: &[Ch]) -> String {
    chars.iter().filter(|c| c.st != Status::Del).map(|c| c.s.as_str()).collect()
}

fn trim_before(s: &str) -> String {
    let n = s.chars().count();
    if n <= CONTEXT {
        return s.to_string();
    }
    let tail: String = s.chars().skip(n - CONTEXT).collect();
    format!("…{}", tail.split_once(' ').map(|(_, r)| r).unwrap_or(&tail))
}

fn trim_after(s: &str) -> String {
    if s.chars().count() <= CONTEXT {
        return s.to_string();
    }
    let head: String = s.chars().take(CONTEXT).collect();
    format!("{}…", head.rsplit_once(' ').map(|(l, _)| l).unwrap_or(&head))
}

fn label(kind: ChangeKind) -> (&'static str, Option<&'static str>) {
    use ChangeKind::*;
    match kind {
        Insert => ("insert", None),
        Delete => ("delete", None),
        MoveTo => ("insert", Some("moved here")),
        MoveFrom => ("delete", Some("moved away")),
        ParagraphInsert => ("insert", Some("new paragraph")),
        ParagraphDelete => ("delete", Some("removed paragraph")),
        RowInsert => ("insert", Some("new table row")),
        RowDelete => ("delete", Some("removed table row")),
        Format => ("format", Some("formatting")),
        ParagraphFormat => ("format", Some("paragraph style")),
        TableFormat => ("format", Some("table layout")),
        SectionFormat => ("format", Some("page setup")),
    }
}

fn part_prefix(part: &str) -> &'static str {
    match part {
        "footnotes" => "f",
        "endnotes" => "e",
        _ => "c",
    }
}

pub fn build(model: &DocModel, changes: &[Change], comments: &[Comment]) -> Review {
    // Paragraphs in reading order, with their section path and part.
    let mut paras: Vec<(ParaInfo, &'static str)> = Vec::new();
    let mut path: Vec<(u8, String)> = Vec::new();
    let add = |p: &MPara, path: &mut Vec<(u8, String)>, part: &'static str, paras: &mut Vec<(ParaInfo, &'static str)>| {
        let chars = para_chars(p);
        if let Some(l) = p.kind.strip_prefix('h').and_then(|n| n.parse::<u8>().ok()) {
            path.retain(|(lv, _)| *lv < l);
            let title = visible(&chars).trim().to_string();
            path.push((l, match &p.label {
                Some(n) => format!("{n} {title}"),
                None => title,
            }));
        }
        let section = path.iter().map(|(_, t)| t.clone()).collect();
        paras.push((ParaInfo { chars, section }, part));
    };
    for b in model.boxes.iter().filter(|b| b.after.is_none()) {
        for p in &b.paras {
            add(p, &mut path, "body", &mut paras);
        }
    }
    for (bi, b) in model.blocks.iter().enumerate() {
        match b {
            MBlock::Para(p) => add(p, &mut path, "body", &mut paras),
            MBlock::Table { rows, .. } => {
                for p in rows.iter().flatten().flatten() {
                    add(p, &mut path, "body", &mut paras);
                }
            }
            MBlock::Opaque { .. } => {}
        }
        for bx in model.boxes.iter().filter(|x| x.after == Some(bi)) {
            for p in &bx.paras {
                add(p, &mut path, "body", &mut paras);
            }
        }
    }
    for n in &model.notes {
        let part = if n.label.starts_with('e') { "endnotes" } else { "footnotes" };
        let mut npath = vec![(1, if part == "endnotes" { "Endnotes".to_string() } else { "Footnotes".to_string() })];
        for p in &n.paras {
            add(p, &mut npath, part, &mut paras);
        }
    }

    // Tracked-change ranges by (part, Word id), and comment anchors.
    let mut ranges: HashMap<(&str, String), Vec<Range>> = HashMap::new();
    let mut anchors: HashMap<String, (usize, usize, String)> = HashMap::new();
    for (pi, (info, part)) in paras.iter().enumerate() {
        let mut i = 0;
        while i < info.chars.len() {
            let c = &info.chars[i];
            if let (Some(w), true) = (&c.wid, c.st != Status::Plain) {
                let mut j = i;
                let mut text = String::new();
                while j < info.chars.len() && info.chars[j].wid.as_ref() == Some(w) && info.chars[j].st == c.st {
                    text.push_str(&info.chars[j].s);
                    j += 1;
                }
                ranges.entry((part, w.clone())).or_default().push(Range { para: pi, start: i, end: j, st: c.st, text });
                i = j;
            } else {
                i += 1;
            }
        }
        for (ci, c) in info.chars.iter().enumerate() {
            for id in &c.comments {
                anchors.entry(id.clone()).or_insert((pi, ci, String::new())).2.push_str(&c.s);
            }
        }
    }

    let context = |para: usize, start: usize, end: usize| -> (String, String) {
        let chars = &paras[para].0.chars;
        (trim_before(&visible(&chars[..start])), trim_after(&visible(&chars[end.min(chars.len())..])))
    };

    let mut items: Vec<Item> = Vec::new();
    for c in changes {
        let part: &str = match c.part.as_str() {
            "footnotes" => "footnotes",
            "endnotes" => "endnotes",
            _ => "body",
        };
        let mut rs: Vec<&Range> = c.wids.iter().flat_map(|w| ranges.get(&(part, w.clone())).into_iter().flatten()).collect();
        rs.sort_by_key(|r| (r.para, r.start));
        let (kind, lbl) = label(c.kind);
        let first_wid = c.wids.first().cloned().unwrap_or_else(|| format!("n{}", c.n));
        let mut it = Item {
            id: format!("{}{}", part_prefix(part), first_wid),
            kind: kind.into(),
            label: lbl.map(String::from),
            author: if c.author.is_empty() { "unknown".into() } else { c.author.clone() },
            date: c.date.clone(),
            section: Vec::new(),
            part: part.into(),
            before: String::new(),
            old: None,
            new: None,
            after: String::new(),
            text: None,
            anchor: None,
            replies: Vec::new(),
            resolved: false,
            wids: c.wids.clone(),
            order: (usize::MAX, 0),
        };
        if let (Some(f), Some(l)) = (rs.first(), rs.last()) {
            it.section = paras[f.para].0.section.clone();
            it.order = (f.para, f.start);
            it.before = context(f.para, f.start, f.end).0;
            it.after = context(l.para, l.start, l.end).1;
            // A table row's cells read "a | b | c".
            let row = matches!(c.kind, ChangeKind::RowInsert | ChangeKind::RowDelete);
            let join = |st: Status| {
                let mut s = String::new();
                let mut last = None;
                for r in rs.iter().filter(|r| r.st == st) {
                    if row && last.is_some_and(|p| p != r.para) {
                        s.push_str(" | ");
                    }
                    s.push_str(&r.text);
                    last = Some(r.para);
                }
                s
            };
            let (ins, del) = (join(Status::Ins), join(Status::Del));
            it.new = Some(ins).filter(|s| !s.is_empty());
            it.old = Some(del).filter(|s| !s.is_empty());
            if it.new.is_some() && it.old.is_some() {
                it.kind = "replace".into();
            }
        }
        if it.old.is_none() && it.new.is_none() {
            it.text = Some(c.text.clone()).filter(|t| !t.is_empty() && t != "¶");
        }
        items.push(it);
    }

    // A deletion directly followed by an insertion by the same person reads as one replacement.
    items.sort_by_key(|i| i.order);
    let mut merged: Vec<Item> = Vec::new();
    for it in items {
        if let Some(prev) = merged.last_mut() {
            let adjacent = prev.kind == "delete"
                && it.kind == "insert"
                && prev.label.is_none()
                && it.label.is_none()
                && prev.author == it.author
                && prev.part == it.part
                && prev.order.0 == it.order.0
                && prev.order.0 != usize::MAX;
            let gap_ok = adjacent && {
                let r = &ranges;
                let pend = prev.wids.iter().flat_map(|w| r.get(&(prev.part.as_str(), w.clone())).into_iter().flatten()).map(|x| x.end).max().unwrap_or(0);
                it.order.1.saturating_sub(pend) <= 1
            };
            if gap_ok {
                prev.kind = "replace".into();
                prev.new = it.new;
                prev.after = it.after;
                prev.wids.extend(it.wids);
                continue;
            }
        }
        merged.push(it);
    }
    let mut items = merged;

    // Comment threads.
    for c in comments.iter().filter(|c| c.parent.is_none()) {
        let at = anchors.get(&c.id);
        let (section, order, before, after) = match at {
            Some((pi, ci, text)) => {
                let (b, a) = context(*pi, *ci, ci + text.chars().count());
                (paras[*pi].0.section.clone(), (*pi, *ci), b, a)
            }
            None => (Vec::new(), (usize::MAX, 0), String::new(), String::new()),
        };
        items.push(Item {
            id: format!("m{}", c.id),
            kind: "comment".into(),
            label: None,
            author: if c.author.is_empty() { "unknown".into() } else { c.author.clone() },
            date: c.date.clone(),
            section,
            part: "body".into(),
            before,
            old: None,
            new: None,
            after,
            text: Some(c.text.clone()),
            anchor: Some(at.map(|a| a.2.clone()).unwrap_or_else(|| c.anchor.clone())).filter(|a| !a.is_empty()),
            replies: comments
                .iter()
                .filter(|r| r.parent.as_deref() == Some(c.id.as_str()))
                .map(|r| Reply { id: format!("m{}", r.id), author: r.author.clone(), date: r.date.clone(), text: r.text.clone() })
                .collect(),
            resolved: c.done,
            wids: Vec::new(),
            order,
        });
    }
    items.sort_by_key(|i| i.order);

    // Summary.
    let mut people: Vec<Person> = Vec::new();
    fn person(people: &mut Vec<Person>, name: &str) -> usize {
        match people.iter().position(|p| p.name == name) {
            Some(i) => i,
            None => {
                people.push(Person { name: name.to_string(), ..Default::default() });
                people.len() - 1
            }
        }
    }
    for it in &items {
        let p = person(&mut people, &it.author);
        match it.kind.as_str() {
            "insert" => people[p].inserts += 1,
            "delete" => people[p].deletes += 1,
            "replace" => people[p].replaces += 1,
            "comment" => people[p].comments += 1,
            _ => people[p].other += 1,
        }
        for r in &it.replies {
            let q = person(&mut people, if r.author.is_empty() { "unknown" } else { &r.author });
            people[q].replies += 1;
        }
    }
    people.sort_by_key(|p| std::cmp::Reverse(p.inserts + p.deletes + p.replaces + p.other + p.comments + p.replies));
    let threads: Vec<&Item> = items.iter().filter(|i| i.kind == "comment").collect();
    let summary = Summary {
        changes: items.len() - threads.len(),
        threads: threads.len(),
        open_threads: threads.iter().filter(|t| !t.resolved).count(),
        people,
    };
    Review { summary, items }
}

impl Review {
    /// The item an id refers to: its own id, or any Word id inside it (`c13`
    /// finds the replacement `c12` it belongs to).
    pub fn find(&self, id: &str) -> Option<&Item> {
        let id = id.trim();
        if let Some(it) = self.items.iter().find(|i| i.id == id) {
            return Some(it);
        }
        let (prefix, num) = id.split_at(id.find(|c: char| c.is_ascii_digit())?);
        let part = match prefix {
            "c" => "body",
            "f" => "footnotes",
            "e" => "endnotes",
            _ => return None,
        };
        self.items.iter().find(|i| i.part == part && i.kind != "comment" && i.wids.iter().any(|w| w == num))
    }
}
