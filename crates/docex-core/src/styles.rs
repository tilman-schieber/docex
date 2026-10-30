//! Maps the document's styles and numbering to the few paragraph kinds docex
//! exposes, and back.

use crate::xml::{Document, Element};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(tag = "kind", content = "level", rename_all = "lowercase")]
pub enum Kind {
    Normal,
    Heading(u8),
    Bullet(u8),
    Number(u8),
    Quote,
    Code,
    /// Figure / table captions (`: Figure 1: ...`).
    Caption,
}

impl Kind {
    pub fn is_list(self) -> bool {
        matches!(self, Kind::Bullet(_) | Kind::Number(_))
    }
}

#[derive(Debug, Default, Clone)]
pub struct StyleInfo {
    pub id: String,
    pub name: String,
    pub based_on: Option<String>,
    pub outline_lvl: Option<u8>,
    pub num_id: Option<String>,
    pub num_lvl: Option<u8>,
    pub is_default: bool,
    pub char_style: bool,
}

#[derive(Debug, Default)]
pub struct Styles {
    pub by_id: HashMap<String, StyleInfo>,
    /// numId -> (ilvl -> is bullet)
    num_bullet: HashMap<String, HashMap<u8, bool>>,
}

impl Styles {
    pub fn load(styles: Option<&Document>, numbering: Option<&Document>) -> Styles {
        let mut s = Styles::default();
        if let Some(doc) = styles {
            for st in doc.root.elements().filter(|e| e.is("w:style")) {
                let ty = st.attr("w:type").unwrap_or("");
                if ty != "paragraph" && ty != "character" {
                    continue;
                }
                let id = st.attr("w:styleId").unwrap_or("").to_string();
                let ppr = st.child("w:pPr");
                let info = StyleInfo {
                    id: id.clone(),
                    name: st
                        .child("w:name")
                        .and_then(|n| n.attr("w:val"))
                        .unwrap_or(&id)
                        .to_lowercase(),
                    based_on: st.child("w:basedOn").and_then(|b| b.attr("w:val")).map(String::from),
                    outline_lvl: ppr
                        .and_then(|p| p.child("w:outlineLvl"))
                        .and_then(|o| o.attr("w:val"))
                        .and_then(|v| v.parse().ok()),
                    num_id: ppr
                        .and_then(|p| p.child_path(&["w:numPr", "w:numId"]))
                        .and_then(|n| n.attr("w:val"))
                        .map(String::from),
                    num_lvl: ppr
                        .and_then(|p| p.child_path(&["w:numPr", "w:ilvl"]))
                        .and_then(|n| n.attr("w:val"))
                        .and_then(|v| v.parse().ok()),
                    is_default: matches!(st.attr("w:default"), Some("1") | Some("true")),
                    char_style: ty == "character",
                };
                s.by_id.insert(id, info);
            }
        }
        if let Some(doc) = numbering {
            let mut abs: HashMap<String, HashMap<u8, bool>> = HashMap::new();
            for a in doc.root.elements().filter(|e| e.is("w:abstractNum")) {
                let mut lv = HashMap::new();
                for l in a.elements().filter(|e| e.is("w:lvl")) {
                    let i: u8 = l.attr("w:ilvl").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let fmt = l.child("w:numFmt").and_then(|f| f.attr("w:val")).unwrap_or("decimal");
                    lv.insert(i, fmt == "bullet" || fmt == "none");
                }
                abs.insert(a.attr("w:abstractNumId").unwrap_or("").to_string(), lv);
            }
            for n in doc.root.elements().filter(|e| e.is("w:num")) {
                let aid = n.child("w:abstractNumId").and_then(|a| a.attr("w:val")).unwrap_or("");
                if let Some(lv) = abs.get(aid) {
                    s.num_bullet.insert(n.attr("w:numId").unwrap_or("").to_string(), lv.clone());
                }
            }
        }
        s
    }

    fn chain(&self, id: &str) -> Vec<&StyleInfo> {
        let mut out = Vec::new();
        let mut cur = self.by_id.get(id);
        while let Some(st) = cur {
            if out.len() > 8 {
                break;
            }
            out.push(st);
            cur = st.based_on.as_deref().and_then(|b| self.by_id.get(b));
        }
        out
    }

    fn kind_from_name(name: &str) -> Option<Kind> {
        let n = name.trim();
        if let Some(rest) = n.strip_prefix("heading") {
            if let Ok(l) = rest.trim().parse::<u8>() {
                return Some(Kind::Heading(l.clamp(1, 6)));
            }
        }
        match n {
            "title" => return Some(Kind::Heading(1)),
            "quote" | "intense quote" | "block text" => return Some(Kind::Quote),
            "source code" | "code" | "html preformatted" | "plain text" => return Some(Kind::Code),
            _ => {}
        }
        if n.ends_with("caption") || n == "beschriftung" {
            return Some(Kind::Caption);
        }
        if n.contains("code") || n.contains("preformatted") {
            return Some(Kind::Code);
        }
        let lvl_of = |rest: &str| rest.trim().parse::<u8>().map(|l| l.saturating_sub(1)).unwrap_or(0);
        if let Some(rest) = n.strip_prefix("list bullet") {
            return Some(Kind::Bullet(lvl_of(rest)));
        }
        if let Some(rest) = n.strip_prefix("list number") {
            return Some(Kind::Number(lvl_of(rest)));
        }
        None
    }

    pub fn is_bullet(&self, num_id: &str, lvl: u8) -> bool {
        self.num_bullet
            .get(num_id)
            .and_then(|m| m.get(&lvl).or_else(|| m.get(&0)))
            .copied()
            .unwrap_or(true)
    }

    /// The list a paragraph is numbered in: numId, level (if given) and the
    /// style the numbering came from (if it came from a style).
    pub fn num_of(&self, ppr: Option<&Element>) -> Option<(String, Option<u8>, Option<String>)> {
        let numpr = ppr.and_then(|p| p.child("w:numPr"));
        let direct = numpr.and_then(|n| n.child("w:numId")).and_then(|n| n.attr("w:val")).map(String::from);
        let mut lvl = numpr.and_then(|n| n.child("w:ilvl")).and_then(|n| n.attr("w:val")).and_then(|v| v.parse::<u8>().ok());
        let style = ppr
            .and_then(|p| p.child("w:pStyle"))
            .and_then(|s| s.attr("w:val"))
            .map(String::from)
            .or_else(|| self.by_id.values().find(|s| s.is_default && !s.char_style).map(|s| s.id.clone()));
        let chain = style.as_deref().map(|s| self.chain(s)).unwrap_or_default();
        let mut from_style = None;
        let mut num = direct;
        for st in &chain {
            if num.is_none() && st.num_id.is_some() {
                num = st.num_id.clone();
                from_style = Some(st.id.clone());
            }
            if lvl.is_none() {
                lvl = st.num_lvl;
            }
        }
        num.filter(|id| id != "0").map(|id| (id, lvl, from_style))
    }

    /// Kind of a paragraph from its `w:pPr`.
    pub fn para_kind(&self, ppr: Option<&Element>) -> Kind {
        let style = ppr
            .and_then(|p| p.child("w:pStyle"))
            .and_then(|s| s.attr("w:val"))
            .map(String::from)
            .or_else(|| self.by_id.values().find(|s| s.is_default && !s.char_style).map(|s| s.id.clone()));
        let chain = style.as_deref().map(|s| self.chain(s)).unwrap_or_default();
        for st in &chain {
            if let Some(k) = Styles::kind_from_name(&st.name) {
                if !k.is_list() {
                    return k;
                }
                break;
            }
            if let Some(l) = st.outline_lvl.filter(|l| *l < 9) {
                return Kind::Heading((l + 1).min(6));
            }
        }
        if let Some(l) = ppr
            .and_then(|p| p.child("w:outlineLvl"))
            .and_then(|o| o.attr("w:val"))
            .and_then(|v| v.parse::<u8>().ok())
            .filter(|l| *l < 9)
        {
            return Kind::Heading((l + 1).min(6));
        }
        // Lists: direct numPr, else numPr inherited from the style.
        let numpr = ppr.and_then(|p| p.child("w:numPr"));
        let mut num_id = numpr
            .and_then(|n| n.child("w:numId"))
            .and_then(|n| n.attr("w:val"))
            .map(String::from);
        let mut lvl = numpr
            .and_then(|n| n.child("w:ilvl"))
            .and_then(|n| n.attr("w:val"))
            .and_then(|v| v.parse::<u8>().ok());
        for st in &chain {
            if num_id.is_none() {
                num_id = st.num_id.clone();
            }
            if lvl.is_none() {
                lvl = st.num_lvl;
            }
        }
        if let Some(id) = num_id.filter(|id| id != "0") {
            let l = lvl.unwrap_or(0);
            return if self.is_bullet(&id, l) { Kind::Bullet(l) } else { Kind::Number(l) };
        }
        for st in &chain {
            if let Some(k) = Styles::kind_from_name(&st.name) {
                return k;
            }
        }
        Kind::Normal
    }

    /// The style id to use for a new paragraph of `kind`, if the document has one.
    pub fn style_for(&self, kind: Kind) -> Option<String> {
        let want: &[&str] = match kind {
            Kind::Normal => return None,
            Kind::Heading(1) => &["heading 1"],
            Kind::Heading(2) => &["heading 2"],
            Kind::Heading(3) => &["heading 3"],
            Kind::Heading(4) => &["heading 4"],
            Kind::Heading(5) => &["heading 5"],
            Kind::Heading(_) => &["heading 6"],
            Kind::Quote => &["quote", "block text", "intense quote"],
            Kind::Code => &["source code", "code", "html preformatted", "plain text"],
            Kind::Caption => &["caption", "image caption", "table caption"],
            Kind::Bullet(_) => &["list bullet", "list paragraph"],
            Kind::Number(_) => &["list number", "list paragraph"],
        };
        for w in want {
            if let Some(st) = self.by_id.values().find(|s| !s.char_style && s.name == *w) {
                return Some(st.id.clone());
            }
        }
        None
    }

    pub fn char_style_named(&self, names: &[&str]) -> Option<String> {
        for n in names {
            if let Some(st) = self.by_id.values().find(|s| s.char_style && s.name == *n) {
                return Some(st.id.clone());
            }
        }
        None
    }

    /// Character style name (lowercase) for an id.
    pub fn name_of(&self, id: &str) -> Option<&str> {
        self.by_id.get(id).map(|s| s.name.as_str())
    }

    /// The first list numId in use for bullets / numbers, from numbering.xml.
    pub fn num_id_for(&self, bullet: bool) -> Option<String> {
        let mut ids: Vec<&String> = self
            .num_bullet
            .iter()
            .filter(|(_, m)| m.get(&0).copied().unwrap_or(true) == bullet)
            .map(|(k, _)| k)
            .collect();
        ids.sort_by_key(|k| k.parse::<u32>().unwrap_or(u32::MAX));
        ids.first().map(|s| s.to_string())
    }
}
