//! Comments: `comments.xml` plus the reply/resolved data in
//! `commentsExtended.xml`.

use crate::package::{Package, Rels};
use crate::xml::{Document, Element, Node, Text};
use anyhow::{bail, Result};

pub const REL_COMMENTS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
pub const REL_COMMENTS_EX: &str = "http://schemas.microsoft.com/office/2011/relationships/commentsExtended";
const CT_COMMENTS: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";
const CT_COMMENTS_EX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml";
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";

#[derive(Clone, Debug, serde::Serialize)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub date: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub done: bool,
    #[serde(skip)]
    pub para_id: Option<String>,
    /// The commented text (filled by the caller from document.xml).
    pub anchor: String,
}

pub struct Comments {
    pub part: String,
    pub doc: Option<Document>,
    pub ext_part: String,
    pub ext: Option<Document>,
    pub list: Vec<Comment>,
}

impl Comments {
    pub fn load(pkg: &Package, rels: &Rels) -> Result<Comments> {
        let part = rels
            .find_by_type("/comments")
            .map(|r| rels.resolve(r.target))
            .unwrap_or_else(|| "word/comments.xml".into());
        let ext_part = rels
            .find_by_type("/commentsExtended")
            .map(|r| rels.resolve(r.target))
            .unwrap_or_else(|| "word/commentsExtended.xml".into());
        let has_rel = rels.find_by_type("/comments").is_some();
        let doc = if has_rel { pkg.read_xml_opt(&part)? } else { None };
        let ext = if rels.find_by_type("/commentsExtended").is_some() { pkg.read_xml_opt(&ext_part)? } else { None };
        let mut c = Comments { part, doc, ext_part, ext, list: Vec::new() };
        c.reindex();
        Ok(c)
    }

    fn reindex(&mut self) {
        self.list.clear();
        let Some(doc) = &self.doc else { return };
        let mut ext: Vec<(String, Option<String>, bool)> = Vec::new();
        if let Some(x) = &self.ext {
            for e in x.root.elements().filter(|e| e.local() == "commentEx") {
                ext.push((
                    e.attr("w15:paraId").unwrap_or("").to_string(),
                    e.attr("w15:paraIdParent").map(String::from),
                    matches!(e.attr("w15:done"), Some("1") | Some("true")),
                ));
            }
        }
        let mut by_para: Vec<(String, String)> = Vec::new();
        for c in doc.root.elements().filter(|e| e.is("w:comment")) {
            let paras: Vec<&Element> = c.elements().filter(|e| e.is("w:p")).collect();
            let text = paras.iter().map(|p| para_text(p)).collect::<Vec<_>>().join("\n");
            let para_id = paras.last().and_then(|p| p.attr("w14:paraId")).map(String::from);
            let id = c.attr("w:id").unwrap_or("").to_string();
            if let Some(pid) = &para_id {
                by_para.push((pid.clone(), id.clone()));
            }
            self.list.push(Comment {
                id,
                author: c.attr("w:author").unwrap_or("").to_string(),
                date: c.attr("w:date").unwrap_or("").to_string(),
                text: text.trim().to_string(),
                parent: None,
                done: false,
                para_id,
                anchor: String::new(),
            });
        }
        for c in &mut self.list {
            if let Some(pid) = &c.para_id {
                if let Some((_, parent, done)) = ext.iter().find(|(p, _, _)| p == pid) {
                    c.done = *done;
                    c.parent = parent
                        .as_ref()
                        .and_then(|pp| by_para.iter().find(|(p, _)| p == pp))
                        .map(|(_, id)| id.clone());
                }
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&Comment> {
        self.list.iter().find(|c| c.id == id)
    }

    pub fn replies(&self, id: &str) -> Vec<&Comment> {
        self.list.iter().filter(|c| c.parent.as_deref() == Some(id)).collect()
    }

    pub fn next_id(&self, document: &Element) -> u32 {
        let mut max: i64 = -1;
        for c in &self.list {
            max = max.max(c.id.parse().unwrap_or(-1));
        }
        document.visit(&mut |e| {
            if e.name.starts_with("w:comment") {
                if let Some(v) = e.attr("w:id").and_then(|v| v.parse::<i64>().ok()) {
                    max = max.max(v);
                }
            }
        });
        (max + 1) as u32
    }

    fn ensure_parts(&mut self, pkg: &mut Package, rels: &mut Rels) -> Result<()> {
        if self.doc.is_none() {
            self.doc = Some(Document::parse(&format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:comments xmlns:w=\"{}\" xmlns:w14=\"{W14}\"></w:comments>",
                crate::package::W_NS
            ))?);
            let target = self.part.strip_prefix("word/").unwrap_or(&self.part).to_string();
            rels.add(REL_COMMENTS, &target, false);
            pkg.ensure_content_type(Some(&self.part), None, CT_COMMENTS)?;
        }
        let root = &mut self.doc.as_mut().unwrap().root;
        if root.attr("xmlns:w14").is_none() {
            root.set_attr("xmlns:w14", W14);
        }
        if self.ext.is_none() {
            self.ext = Some(Document::parse(&format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w15:commentsEx xmlns:w15=\"{W15}\"></w15:commentsEx>"
            ))?);
            let target = self.ext_part.strip_prefix("word/").unwrap_or(&self.ext_part).to_string();
            rels.add(REL_COMMENTS_EX, &target, false);
            pkg.ensure_content_type(Some(&self.ext_part), None, CT_COMMENTS_EX)?;
        }
        Ok(())
    }

    /// Add a comment entry (the anchors in document.xml are the caller's job).
    pub fn add(
        &mut self,
        pkg: &mut Package,
        rels: &mut Rels,
        id: u32,
        author: &str,
        date: &str,
        text: &str,
        parent: Option<&str>,
    ) -> Result<()> {
        self.ensure_parts(pkg, rels)?;
        let parent_pid = match parent {
            Some(p) => match self.get(p).and_then(|c| c.para_id.clone()) {
                Some(pid) => Some(pid),
                None => bail!("comment {p} has no paragraph id; cannot reply to it"),
            },
            None => None,
        };
        let pid = format!("{:08X}", 0x1000_0000u32 + (fnv(&format!("{id}{date}{text}")) as u32 & 0x6FFF_FFFF));
        let initials: String = author.split_whitespace().filter_map(|w| w.chars().next()).collect();
        let mut c = Element::new("w:comment")
            .with_attr("w:id", id.to_string())
            .with_attr("w:author", author)
            .with_attr("w:date", date)
            .with_attr("w:initials", initials);
        let lines: Vec<&str> = text.split('\n').collect();
        for (i, line) in lines.iter().enumerate() {
            let mut p = Element::new("w:p");
            if i == lines.len() - 1 {
                p.set_attr("w14:paraId", pid.clone());
                p.set_attr("w14:textId", "77777777");
            }
            if i == 0 {
                p.children.push(
                    Element::new("w:r")
                        .with_child(Element::new("w:rPr").with_child(Element::new("w:rStyle").with_attr("w:val", "CommentReference")))
                        .with_child(Element::new("w:annotationRef"))
                        .into(),
                );
            }
            p.children.push(Element::new("w:r").with_child(text_el(line)).into());
            c.children.push(p.into());
        }
        self.doc.as_mut().unwrap().root.children.push(c.into());
        let mut ex = Element::new("w15:commentEx").with_attr("w15:paraId", pid);
        if let Some(pp) = parent_pid {
            ex.set_attr("w15:paraIdParent", pp);
        }
        ex.set_attr("w15:done", "0");
        self.ext.as_mut().unwrap().root.children.push(ex.into());
        self.reindex();
        Ok(())
    }

    pub fn set_done(&mut self, pkg: &mut Package, rels: &mut Rels, id: &str, done: bool) -> Result<()> {
        let Some(pid) = self.get(id).map(|c| c.para_id.clone()) else { bail!("no comment with id {id}") };
        let Some(pid) = pid else { bail!("comment {id} has no paragraph id (old Word format); cannot resolve") };
        self.ensure_parts(pkg, rels)?;
        let root = &mut self.ext.as_mut().unwrap().root;
        let val = if done { "1" } else { "0" };
        if let Some(e) = root.elements_mut().find(|e| e.local() == "commentEx" && e.attr("w15:paraId") == Some(&pid)) {
            e.set_attr("w15:done", val);
        } else {
            root.children.push(Element::new("w15:commentEx").with_attr("w15:paraId", pid).with_attr("w15:done", val).into());
        }
        self.reindex();
        Ok(())
    }

    pub fn remove(&mut self, id: &str) {
        let pid = self.get(id).and_then(|c| c.para_id.clone());
        if let Some(doc) = &mut self.doc {
            doc.root.children.retain(|c| !matches!(c, Node::Element(e) if e.is("w:comment") && e.attr("w:id") == Some(id)));
        }
        if let (Some(x), Some(pid)) = (&mut self.ext, pid) {
            x.root.children.retain(|c| !matches!(c, Node::Element(e) if e.attr("w15:paraId") == Some(pid.as_str())));
        }
        self.reindex();
    }

    pub fn save(&self, pkg: &mut Package) {
        if let Some(d) = &self.doc {
            pkg.write_xml(&self.part, d);
        }
        if let Some(x) = &self.ext {
            pkg.write_xml(&self.ext_part, x);
        }
    }
}

fn para_text(p: &Element) -> String {
    let mut s = String::new();
    p.visit(&mut |e| match e.name.as_str() {
        "w:t" => s.push_str(&e.text_content()),
        "w:tab" => s.push('\t'),
        "w:br" => s.push('\n'),
        _ => {}
    });
    s
}

pub fn text_el(s: &str) -> Element {
    let mut t = Element::new("w:t");
    if s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace) || s.contains("  ") {
        t.set_attr("xml:space", "preserve");
    }
    t.children.push(Node::Text(Text::new(s)));
    t
}

pub fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
