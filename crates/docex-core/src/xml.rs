//! A minimal lossless XML DOM.
//!
//! OOXML parts are plain XML without DTDs, so a small hand-written tokenizer is
//! enough. Every node remembers the exact source bytes it was parsed from;
//! serializing an untouched tree reproduces the input byte for byte. Mutating
//! an element's attributes drops its raw start tag so it is regenerated.

use anyhow::{bail, Result};
use std::fmt::Write as _;

#[derive(Clone, Debug)]
pub enum Node {
    Element(Element),
    Text(Text),
    /// Declarations, comments, processing instructions, CDATA: kept verbatim.
    Other(String),
}

#[derive(Clone, Debug)]
pub struct Text {
    pub text: String,
    raw: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Element {
    pub name: String,
    attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    raw_open: Option<String>,
    raw_close: Option<String>,
    /// The raw open tag was a self-closing `<x/>` tag.
    raw_self_closing: bool,
}

#[derive(Clone, Debug)]
pub struct Document {
    /// Everything before the root element (XML declaration, whitespace).
    pub prolog: Vec<Node>,
    pub root: Element,
    pub epilog: Vec<Node>,
}

impl Text {
    pub fn new(s: impl Into<String>) -> Self {
        Text { text: s.into(), raw: None }
    }
}

impl Element {
    pub fn new(name: impl Into<String>) -> Self {
        Element {
            name: name.into(),
            attrs: Vec::new(),
            children: Vec::new(),
            raw_open: None,
            raw_close: None,
            raw_self_closing: false,
        }
    }

    pub fn with_attr(mut self, k: &str, v: impl Into<String>) -> Self {
        self.set_attr(k, v);
        self
    }

    pub fn with_child(mut self, c: impl Into<Node>) -> Self {
        self.children.push(c.into());
        self
    }

    pub fn attrs(&self) -> &[(String, String)] {
        &self.attrs
    }

    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    pub fn set_attr(&mut self, k: &str, v: impl Into<String>) {
        let v = v.into();
        self.raw_open = None;
        self.raw_self_closing = false;
        if let Some(a) = self.attrs.iter_mut().find(|(n, _)| n == k) {
            a.1 = v;
        } else {
            self.attrs.push((k.to_string(), v));
        }
    }

    pub fn remove_attr(&mut self, k: &str) {
        if self.attrs.iter().any(|(n, _)| n == k) {
            self.raw_open = None;
            self.raw_self_closing = false;
            self.attrs.retain(|(n, _)| n != k);
        }
    }

    pub fn rename(&mut self, name: &str) {
        self.name = name.to_string();
        self.raw_open = None;
        self.raw_close = None;
        self.raw_self_closing = false;
    }

    /// Insert `child`, replacing any same-named child, at the position the
    /// schema's child `order` requires.
    pub fn insert_ordered(&mut self, child: Element, order: &[&str]) {
        self.remove_children(&child.name);
        let rank = |n: &str| order.iter().position(|o| *o == n).unwrap_or(order.len());
        let r = rank(&child.name);
        let pos = self
            .children
            .iter()
            .position(|c| matches!(c, Node::Element(e) if rank(&e.name) > r))
            .unwrap_or(self.children.len());
        self.children.insert(pos, child.into());
    }

    /// A copy without children (keeps the raw start tag when possible).
    pub fn shell(&self) -> Element {
        let mut e = self.clone();
        e.children.clear();
        e
    }

    pub fn local(&self) -> &str {
        local_name(&self.name)
    }

    pub fn is(&self, name: &str) -> bool {
        self.name == name
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|c| match c {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }

    pub fn child_mut(&mut self, name: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.name == name)
    }

    /// Path lookup like `child_path(&["w:pPr", "w:pStyle"])`.
    pub fn child_path(&self, path: &[&str]) -> Option<&Element> {
        let mut cur = self;
        for p in path {
            cur = cur.child(p)?;
        }
        Some(cur)
    }

    pub fn remove_children(&mut self, name: &str) {
        self.children
            .retain(|c| !matches!(c, Node::Element(e) if e.name == name));
    }

    /// Depth-first search for all descendants with the given name.
    pub fn descendants<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for e in self.elements() {
            if e.name == name {
                out.push(e);
            }
            e.descendants(name, out);
        }
    }

    pub fn visit<'a>(&'a self, f: &mut dyn FnMut(&'a Element)) {
        f(self);
        for e in self.elements() {
            e.visit(f);
        }
    }

    pub fn visit_mut(&mut self, f: &mut dyn FnMut(&mut Element)) {
        f(self);
        for e in self.elements_mut() {
            e.visit_mut(f);
        }
    }

    /// Concatenated text of all text nodes below this element.
    pub fn text_content(&self) -> String {
        let mut s = String::new();
        fn rec(e: &Element, s: &mut String) {
            for c in &e.children {
                match c {
                    Node::Text(t) => s.push_str(&t.text),
                    Node::Element(e) => rec(e, s),
                    Node::Other(_) => {}
                }
            }
        }
        rec(self, &mut s);
        s
    }

    pub fn write_to(&self, out: &mut String) {
        let use_raw_empty = self.children.is_empty() && self.raw_self_closing;
        match (&self.raw_open, use_raw_empty) {
            (Some(raw), true) => {
                out.push_str(raw);
                return;
            }
            (Some(raw), false) if !self.raw_self_closing => out.push_str(raw),
            _ => {
                out.push('<');
                out.push_str(&self.name);
                for (k, v) in &self.attrs {
                    let _ = write!(out, " {}=\"{}\"", k, escape_attr(v));
                }
                if self.children.is_empty() {
                    out.push_str("/>");
                    return;
                }
                out.push('>');
            }
        }
        for c in &self.children {
            c.write_to(out);
        }
        match &self.raw_close {
            Some(raw) => out.push_str(raw),
            None => {
                out.push_str("</");
                out.push_str(&self.name);
                out.push('>');
            }
        }
    }

    pub fn to_xml(&self) -> String {
        let mut s = String::new();
        self.write_to(&mut s);
        s
    }
}

impl Node {
    pub fn write_to(&self, out: &mut String) {
        match self {
            Node::Element(e) => e.write_to(out),
            Node::Text(t) => match &t.raw {
                Some(r) => out.push_str(r),
                None => out.push_str(&escape_text(&t.text)),
            },
            Node::Other(s) => out.push_str(s),
        }
    }

    pub fn as_element(&self) -> Option<&Element> {
        match self {
            Node::Element(e) => Some(e),
            _ => None,
        }
    }

    pub fn as_element_mut(&mut self) -> Option<&mut Element> {
        match self {
            Node::Element(e) => Some(e),
            _ => None,
        }
    }
}

impl From<Element> for Node {
    fn from(e: Element) -> Self {
        Node::Element(e)
    }
}

impl From<Text> for Node {
    fn from(t: Text) -> Self {
        Node::Text(t)
    }
}

impl Document {
    pub fn parse(src: &str) -> Result<Document> {
        let mut p = Parser { s: src, pos: 0 };
        let mut prolog = Vec::new();
        loop {
            p.skip_bom();
            match p.next_token()? {
                Some(Tok::Start { name, attrs, raw, empty }) => {
                    let root = p.element(name, attrs, raw, empty)?;
                    let mut epilog = Vec::new();
                    while let Some(t) = p.next_token()? {
                        match t {
                            Tok::Text(raw) => epilog.push(Node::Text(Text {
                                text: unescape(raw),
                                raw: Some(raw.to_string()),
                            })),
                            Tok::Other(raw) => epilog.push(Node::Other(raw.to_string())),
                            _ => bail!("content after root element"),
                        }
                    }
                    return Ok(Document { prolog, root, epilog });
                }
                Some(Tok::Text(raw)) | Some(Tok::Other(raw)) => {
                    prolog.push(Node::Other(raw.to_string()))
                }
                Some(Tok::End { .. }) => bail!("unexpected end tag"),
                None => bail!("no root element"),
            }
        }
    }

    pub fn to_xml(&self) -> String {
        let mut s = String::new();
        for n in &self.prolog {
            n.write_to(&mut s);
        }
        self.root.write_to(&mut s);
        for n in &self.epilog {
            n.write_to(&mut s);
        }
        s
    }
}

pub fn local_name(name: &str) -> &str {
    name.rsplit_once(':').map(|(_, l)| l).unwrap_or(name)
}

enum Tok<'a> {
    Start { name: &'a str, attrs: Vec<(String, String)>, raw: &'a str, empty: bool },
    End { name: &'a str, raw: &'a str },
    Text(&'a str),
    Other(&'a str),
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn skip_bom(&mut self) {
        if self.pos == 0 && self.s.starts_with('\u{feff}') {
            self.pos = 3;
        }
    }

    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }

    fn take_until(&mut self, pat: &str) -> Result<&'a str> {
        let start = self.pos;
        match self.rest().find(pat) {
            Some(i) => {
                self.pos += i + pat.len();
                Ok(&self.s[start..self.pos])
            }
            None => bail!("unterminated construct at byte {start}"),
        }
    }

    fn next_token(&mut self) -> Result<Option<Tok<'a>>> {
        let rest = self.rest();
        if rest.is_empty() {
            return Ok(None);
        }
        if !rest.starts_with('<') {
            let start = self.pos;
            let len = rest.find('<').unwrap_or(rest.len());
            self.pos += len;
            return Ok(Some(Tok::Text(&self.s[start..self.pos])));
        }
        if rest.starts_with("<!--") {
            return Ok(Some(Tok::Other(self.take_until("-->")?)));
        }
        if rest.starts_with("<![CDATA[") {
            return Ok(Some(Tok::Other(self.take_until("]]>")?)));
        }
        if rest.starts_with("<?") {
            return Ok(Some(Tok::Other(self.take_until("?>")?)));
        }
        if rest.starts_with("<!") {
            return Ok(Some(Tok::Other(self.take_until(">")?)));
        }
        if rest.starts_with("</") {
            let raw = self.take_until(">")?;
            let name = raw[2..raw.len() - 1].trim();
            return Ok(Some(Tok::End { name, raw }));
        }
        // Start tag: scan to '>' while respecting quoted attribute values.
        let start = self.pos;
        let bytes = self.s.as_bytes();
        let mut i = self.pos + 1;
        let mut quote = 0u8;
        while i < bytes.len() {
            let b = bytes[i];
            if quote != 0 {
                if b == quote {
                    quote = 0;
                }
            } else if b == b'"' || b == b'\'' {
                quote = b;
            } else if b == b'>' {
                break;
            }
            i += 1;
        }
        if i >= bytes.len() {
            bail!("unterminated start tag at byte {start}");
        }
        self.pos = i + 1;
        let raw = &self.s[start..self.pos];
        let empty = raw.ends_with("/>");
        let inner = &raw[1..raw.len() - if empty { 2 } else { 1 }];
        let name_end = inner
            .find(|c: char| c.is_whitespace())
            .unwrap_or(inner.len());
        let name = &inner[..name_end];
        let attrs = parse_attrs(&inner[name_end..])?;
        Ok(Some(Tok::Start { name, attrs, raw, empty }))
    }

    fn element(
        &mut self,
        name: &'a str,
        attrs: Vec<(String, String)>,
        raw: &'a str,
        empty: bool,
    ) -> Result<Element> {
        let mut el = Element {
            name: name.to_string(),
            attrs,
            children: Vec::new(),
            raw_open: Some(raw.to_string()),
            raw_close: None,
            raw_self_closing: empty,
        };
        if empty {
            return Ok(el);
        }
        loop {
            match self.next_token()? {
                Some(Tok::Start { name, attrs, raw, empty }) => {
                    let c = self.element(name, attrs, raw, empty)?;
                    el.children.push(Node::Element(c));
                }
                Some(Tok::End { name: n, raw }) => {
                    if n != el.name {
                        bail!("mismatched end tag </{n}> for <{}>", el.name);
                    }
                    el.raw_close = Some(raw.to_string());
                    return Ok(el);
                }
                Some(Tok::Text(raw)) => el.children.push(Node::Text(Text {
                    text: unescape(raw),
                    raw: Some(raw.to_string()),
                })),
                Some(Tok::Other(raw)) => {
                    if let Some(cdata) = raw
                        .strip_prefix("<![CDATA[")
                        .and_then(|r| r.strip_suffix("]]>"))
                    {
                        el.children.push(Node::Text(Text {
                            text: cdata.to_string(),
                            raw: Some(raw.to_string()),
                        }));
                    } else {
                        el.children.push(Node::Other(raw.to_string()));
                    }
                }
                None => bail!("unexpected end of input inside <{}>", el.name),
            }
        }
    }
}

fn parse_attrs(s: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while !rest.is_empty() {
        let eq = rest.find('=').ok_or_else(|| anyhow::anyhow!("bad attribute: {rest}"))?;
        let key = rest[..eq].trim().to_string();
        rest = rest[eq + 1..].trim_start();
        let q = rest.chars().next().ok_or_else(|| anyhow::anyhow!("bad attribute value"))?;
        if q != '"' && q != '\'' {
            bail!("unquoted attribute value for {key}");
        }
        let end = rest[1..].find(q).ok_or_else(|| anyhow::anyhow!("unterminated attribute"))?;
        out.push((key, unescape(&rest[1..1 + end])));
        rest = rest[end + 2..].trim_start();
    }
    Ok(out)
}

pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_exact() {
        let src = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<w:document xmlns:w=\"x\"><w:body><w:p w:rsidR='00A1'><w:r><w:t xml:space=\"preserve\"> a &amp; b &#x41; </w:t></w:r><w:p /><!-- c --></w:p></w:body></w:document>";
        let d = Document::parse(src).unwrap();
        assert_eq!(d.to_xml(), src);
    }

    #[test]
    fn regenerate_after_mutation() {
        let d = Document::parse("<a x='1'><b/></a>").unwrap();
        let mut r = d.root.clone();
        r.set_attr("y", "2&");
        assert_eq!(r.to_xml(), "<a x=\"1\" y=\"2&amp;\"><b/></a>");
        let mut b = r.child("b").unwrap().clone();
        b.children.push(Node::Text(Text::new("t")));
        assert_eq!(b.to_xml(), "<b>t</b>");
    }
}
