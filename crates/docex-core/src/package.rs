//! The .docx zip package. Parts that are not modified are copied raw, so
//! their bytes (and compression) are exactly what the other side sent.

use crate::xml::{Document, Element};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

pub struct Package {
    bytes: Vec<u8>,
    /// Part names in original archive order.
    order: Vec<String>,
    /// Parts replaced or added since loading.
    changed: BTreeMap<String, Vec<u8>>,
    removed: Vec<String>,
}

pub const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

impl Package {
    pub fn open(path: &std::path::Path) -> Result<Package> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Package::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Package> {
        let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).context("not a zip / docx file")?;
        let mut order = Vec::new();
        for i in 0..zip.len() {
            order.push(zip.by_index_raw(i)?.name()?.to_string());
        }
        Ok(Package { bytes, order, changed: BTreeMap::new(), removed: Vec::new() })
    }

    pub fn has(&self, name: &str) -> bool {
        self.changed.contains_key(name)
            || (self.order.iter().any(|n| n == name) && !self.removed.iter().any(|n| n == name))
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        if let Some(b) = self.changed.get(name) {
            return Ok(b.clone());
        }
        let mut zip = zip::ZipArchive::new(Cursor::new(&self.bytes))?;
        let mut f = zip.by_name(name).with_context(|| format!("missing part {name}"))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Ok(buf)
    }

    pub fn read_string(&self, name: &str) -> Result<String> {
        String::from_utf8(self.read(name)?).with_context(|| format!("{name} is not UTF-8"))
    }

    pub fn read_xml(&self, name: &str) -> Result<Document> {
        Document::parse(&self.read_string(name)?).with_context(|| format!("parsing {name}"))
    }

    pub fn read_xml_opt(&self, name: &str) -> Result<Option<Document>> {
        if self.has(name) { self.read_xml(name).map(Some) } else { Ok(None) }
    }

    pub fn write(&mut self, name: &str, data: Vec<u8>) {
        self.removed.retain(|n| n != name);
        self.changed.insert(name.to_string(), data);
    }

    pub fn write_xml(&mut self, name: &str, doc: &Document) {
        self.write(name, doc.to_xml().into_bytes());
    }

    pub fn is_changed(&self) -> bool {
        !self.changed.is_empty() || !self.removed.is_empty()
    }

    pub fn part_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .order
            .iter()
            .filter(|n| !self.removed.contains(n))
            .cloned()
            .collect();
        for k in self.changed.keys() {
            if !v.contains(k) {
                v.push(k.clone());
            }
        }
        v
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if !self.is_changed() {
            return Ok(self.bytes.clone());
        }
        let mut src = zip::ZipArchive::new(Cursor::new(&self.bytes))?;
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (i, name) in self.order.iter().enumerate() {
            if self.removed.contains(name) {
                continue;
            }
            if let Some(data) = self.changed.get(name) {
                out.start_file(name.as_str(), opts)?;
                out.write_all(data)?;
            } else {
                out.raw_copy_file(src.by_index_raw(i)?)?;
            }
        }
        for (name, data) in &self.changed {
            if !self.order.contains(name) {
                out.start_file(name.as_str(), opts)?;
                out.write_all(data)?;
            }
        }
        Ok(out.finish()?.into_inner())
    }

    /// Path of the main document part (usually `word/document.xml`).
    pub fn main_part(&self) -> Result<String> {
        let rels = self.read_xml("_rels/.rels")?;
        for r in rels.root.elements() {
            if r.attr("Type").is_some_and(|t| t.ends_with("/officeDocument")) {
                let t = r.attr("Target").unwrap_or("word/document.xml");
                return Ok(t.trim_start_matches('/').to_string());
            }
        }
        Ok("word/document.xml".into())
    }

    /// Ensure `[Content_Types].xml` covers `part` (override) or `ext` (default).
    pub fn ensure_content_type(&mut self, part: Option<&str>, ext: Option<&str>, ct: &str) -> Result<()> {
        let mut doc = self.read_xml("[Content_Types].xml")?;
        let root = &mut doc.root;
        if let Some(ext) = ext {
            if root.elements().any(|e| e.local() == "Default" && e.attr("Extension").is_some_and(|x| x.eq_ignore_ascii_case(ext))) {
                return Ok(());
            }
            root.children.insert(0, Element::new("Default").with_attr("Extension", ext).with_attr("ContentType", ct).into());
        }
        if let Some(part) = part {
            let pn = format!("/{part}");
            if root.elements().any(|e| e.local() == "Override" && e.attr("PartName") == Some(&pn)) {
                return Ok(());
            }
            root.children.push(Element::new("Override").with_attr("PartName", pn).with_attr("ContentType", ct).into());
        }
        self.write_xml("[Content_Types].xml", &doc);
        Ok(())
    }
}

/// Relationships of one part, e.g. `word/_rels/document.xml.rels`.
pub struct Rels {
    pub part: String,
    pub doc: Document,
}

pub struct Rel<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub target: &'a str,
    pub external: bool,
}

impl Rels {
    pub fn rels_path(part: &str) -> String {
        match part.rsplit_once('/') {
            Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
            None => format!("_rels/{part}.rels"),
        }
    }

    pub fn load(pkg: &Package, part: &str) -> Result<Rels> {
        let path = Rels::rels_path(part);
        let doc = match pkg.read_xml_opt(&path)? {
            Some(d) => d,
            None => Document::parse(&format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"{REL_NS}\"></Relationships>"
            ))?,
        };
        Ok(Rels { part: part.to_string(), doc })
    }

    pub fn iter(&self) -> impl Iterator<Item = Rel<'_>> {
        self.doc.root.elements().map(|e| Rel {
            id: e.attr("Id").unwrap_or(""),
            kind: e.attr("Type").unwrap_or(""),
            target: e.attr("Target").unwrap_or(""),
            external: e.attr("TargetMode") == Some("External"),
        })
    }

    pub fn get(&self, id: &str) -> Option<Rel<'_>> {
        self.iter().find(|r| r.id == id)
    }

    /// Target resolved to a package part name (for internal targets).
    pub fn resolve(&self, target: &str) -> String {
        if let Some(t) = target.strip_prefix('/') {
            return t.to_string();
        }
        let base = self.part.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let mut parts: Vec<&str> = if base.is_empty() { vec![] } else { base.split('/').collect() };
        for seg in target.split('/') {
            match seg {
                ".." => {
                    parts.pop();
                }
                "." | "" => {}
                s => parts.push(s),
            }
        }
        parts.join("/")
    }

    pub fn find_by_type(&self, suffix: &str) -> Option<Rel<'_>> {
        self.iter().find(|r| r.kind.ends_with(suffix))
    }

    pub fn add(&mut self, kind: &str, target: &str, external: bool) -> String {
        let mut n = 1;
        let id = loop {
            let id = format!("rId{n}");
            if self.get(&id).is_none() {
                break id;
            }
            n += 1;
        };
        let mut e = Element::new("Relationship")
            .with_attr("Id", id.clone())
            .with_attr("Type", kind)
            .with_attr("Target", target);
        if external {
            e.set_attr("TargetMode", "External");
        }
        self.doc.root.children.push(e.into());
        id
    }

    pub fn save(&self, pkg: &mut Package) {
        pkg.write_xml(&Rels::rels_path(&self.part), &self.doc);
    }
}
