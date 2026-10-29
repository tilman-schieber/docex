//! The projection: a read-only view of `document.xml` as blocks of items,
//! each item tied back to the XML it came from so edits can be patched in.

use crate::package::Rels;
use crate::styles::{Kind, Styles};
use crate::xml::{Element, Node};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize)]
pub struct Marks {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapKind {
    Ins,
    Del,
    Hyperlink,
    /// smartTag, customXml, fldSimple, sdt, ... (content passes through)
    Other,
}

#[derive(Clone, Debug)]
pub struct Wrap {
    pub kind: WrapKind,
    /// The wrapper element without its content children.
    pub shell: Element,
    /// For `w:sdt`: content lives inside this child element.
    pub content_child: Option<Element>,
    pub author: Option<String>,
    pub link: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RunInfo {
    pub shell: Element,
    pub rpr: Option<Element>,
    pub marks: Marks,
}

#[derive(Clone, Debug)]
pub enum What {
    Char(char),
    /// A visible non-text object: tab, break, image, footnote ref, ...
    Obj { key: String, node: Node },
    /// Invisible markup kept in place: bookmarks, comment anchors, field codes.
    Hidden(Node),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub ctx: Vec<usize>,
    pub run: Option<usize>,
    pub what: What,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Plain,
    Ins,
    Del,
}

#[derive(Clone, Debug)]
pub struct Para {
    /// Child-index path from `w:body`.
    pub path: Vec<usize>,
    pub shell: Element,
    pub ppr: Option<Element>,
    pub kind: Kind,
    pub items: Vec<Item>,
    pub runs: Vec<RunInfo>,
    pub wraps: Vec<Wrap>,
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub paras: Vec<Para>,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub child_idx: usize,
    pub cells: Vec<Cell>,
    pub cell_idx: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct Table {
    pub idx: usize,
    pub rows: Vec<Row>,
    pub simple: bool,
}

/// A footnote or endnote, labelled as in Markdown (`3`, endnotes `e3`).
#[derive(Clone, Debug)]
pub struct Note {
    pub label: String,
    pub paras: Vec<Para>,
}

/// A text box (Word "Textfeld"): its paragraphs and where its content lives.
#[derive(Clone, Debug)]
pub struct TextBox {
    pub id: String,
    /// Body index of the block the box is anchored in.
    pub anchor: usize,
    /// Paths from `w:body` to each copy of the box content (`w:txbxContent`;
    /// modern and VML fallback). Paragraph paths are relative to the first.
    pub targets: Vec<Vec<usize>>,
    pub paras: Vec<Para>,
}

#[derive(Clone, Debug)]
pub enum Block {
    Para(Para),
    Table(Table),
    Opaque { idx: usize, label: String },
}

impl Block {
    pub fn body_idx(&self) -> usize {
        match self {
            Block::Para(p) => p.path[0],
            Block::Table(t) => t.idx,
            Block::Opaque { idx, .. } => *idx,
        }
    }
}

impl Item {
    pub fn key(&self) -> Option<String> {
        match &self.what {
            What::Char(c) => Some(c.to_string()),
            What::Obj { key, .. } => Some(key.clone()),
            What::Hidden(_) => None,
        }
    }

    pub fn is_visible(&self) -> bool {
        !matches!(self.what, What::Hidden(_))
    }

    pub fn is_space(&self) -> bool {
        match &self.what {
            What::Char(c) => c.is_whitespace(),
            What::Obj { key, .. } => key == "\t" || key == "\n",
            _ => false,
        }
    }
}

impl Para {
    pub fn status(&self, item: &Item) -> Status {
        for w in item.ctx.iter().rev() {
            match self.wraps[*w].kind {
                WrapKind::Ins => return Status::Ins,
                WrapKind::Del => return Status::Del,
                _ => {}
            }
        }
        Status::Plain
    }

    /// Author of the innermost tracked-change wrapper of an item.
    pub fn change_author(&self, item: &Item) -> Option<&str> {
        item.ctx
            .iter()
            .rev()
            .map(|w| &self.wraps[*w])
            .find(|w| matches!(w.kind, WrapKind::Ins | WrapKind::Del))
            .and_then(|w| w.author.as_deref())
    }

    pub fn link(&self, item: &Item) -> Option<&str> {
        item.ctx
            .iter()
            .rev()
            .find_map(|w| self.wraps[*w].link.as_deref())
    }

    pub fn marks(&self, item: &Item) -> Marks {
        item.run.map(|r| self.runs[r].marks).unwrap_or_default()
    }

    pub fn visible(&self) -> impl Iterator<Item = (usize, &Item)> {
        self.items.iter().enumerate().filter(|(_, i)| i.is_visible())
    }

    pub fn is_empty(&self) -> bool {
        !self.items.iter().any(|i| i.is_visible() && !i.is_space())
    }

    pub fn plain_text(&self) -> String {
        self.visible()
            .filter(|(_, i)| self.status(i) != Status::Del)
            .filter_map(|(_, i)| match &i.what {
                What::Char(c) => Some(c.to_string()),
                What::Obj { key, .. } if key == "\t" || key == "\n" => Some(key.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn has_sect_pr(&self) -> bool {
        self.ppr.as_ref().is_some_and(|p| p.child("w:sectPr").is_some())
    }
}

pub struct Ctx<'a> {
    pub styles: &'a Styles,
    pub rels: &'a Rels,
}

fn toggle_on(e: Option<&Element>) -> bool {
    match e {
        None => false,
        Some(e) => !matches!(e.attr("w:val"), Some("0") | Some("false") | Some("off")),
    }
}

const MONO_FONTS: &[&str] = &[
    "courier new", "courier", "consolas", "menlo", "monaco", "source code pro",
    "dejavu sans mono", "liberation mono", "lucida console", "fira code", "fira mono",
    "jetbrains mono", "cascadia code", "cascadia mono",
];

pub fn marks_of(rpr: Option<&Element>, styles: &Styles) -> Marks {
    let Some(rpr) = rpr else { return Marks::default() };
    let mut m = Marks {
        bold: toggle_on(rpr.child("w:b")),
        italic: toggle_on(rpr.child("w:i")),
        strike: toggle_on(rpr.child("w:strike")) || toggle_on(rpr.child("w:dstrike")),
        code: false,
    };
    if let Some(st) = rpr.child("w:rStyle").and_then(|s| s.attr("w:val")) {
        let name = styles.name_of(st).unwrap_or(st).to_lowercase();
        if name.contains("verbatim") || name.contains("code") {
            m.code = true;
        }
        if name == "strong" {
            m.bold = true;
        }
        if name == "emphasis" {
            m.italic = true;
        }
    }
    if let Some(f) = rpr.child("w:rFonts").and_then(|f| f.attr("w:ascii").or(f.attr("w:hAnsi"))) {
        if MONO_FONTS.contains(&f.to_lowercase().as_str()) {
            m.code = true;
        }
    }
    m
}

/// Build the block list for the body element.
pub fn project(body: &Element, cx: &Ctx) -> Vec<Block> {
    let mut out = Vec::new();
    for (i, c) in body.children.iter().enumerate() {
        let Node::Element(e) = c else { continue };
        match e.name.as_str() {
            "w:p" => out.push(Block::Para(para(e, vec![i], cx))),
            "w:tbl" => out.push(Block::Table(table(e, i, cx))),
            "w:sectPr" | "w:bookmarkStart" | "w:bookmarkEnd" | "w:proofErr" | "w:permStart"
            | "w:permEnd" | "w:commentRangeStart" | "w:commentRangeEnd" | "w:moveFromRangeStart"
            | "w:moveFromRangeEnd" | "w:moveToRangeStart" | "w:moveToRangeEnd" => {}
            _ => out.push(Block::Opaque { idx: i, label: opaque_label(e) }),
        }
    }
    out
}

/// Paragraphs of a container element (`w:txbxContent`, `w:footnote`), with
/// paths relative to `prefix`.
pub fn container_paras(e: &Element, prefix: &[usize], cx: &Ctx) -> Vec<Para> {
    e.children
        .iter()
        .enumerate()
        .filter_map(|(i, c)| match c {
            Node::Element(p) if p.is("w:p") => {
                let mut path = prefix.to_vec();
                path.push(i);
                Some(para(p, path, cx))
            }
            _ => None,
        })
        .collect()
}

/// All text boxes in the body, in document order.
pub fn text_boxes(body: &Element, cx: &Ctx) -> Vec<TextBox> {
    fn contents(e: &Element, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        for (i, c) in e.children.iter().enumerate() {
            let Node::Element(c) = c else { continue };
            path.push(i);
            if c.is("w:txbxContent") {
                out.push(path.clone());
            } else {
                contents(c, path, out);
            }
            path.pop();
        }
    }
    fn objects(e: &Element, path: &mut Vec<usize>, anchor: usize, out: &mut Vec<(usize, Vec<Vec<usize>>)>) {
        for (i, c) in e.children.iter().enumerate() {
            let Node::Element(c) = c else { continue };
            path.push(i);
            if matches!(c.name.as_str(), "mc:AlternateContent" | "w:drawing" | "w:pict") {
                let mut targets = Vec::new();
                let mut sub = path.clone();
                contents(c, &mut sub, &mut targets);
                if !targets.is_empty() {
                    out.push((anchor, targets));
                }
            } else if !c.is("w:txbxContent") {
                objects(c, path, anchor, out);
            }
            path.pop();
        }
    }
    let mut found = Vec::new();
    for (i, c) in body.children.iter().enumerate() {
        if let Node::Element(e) = c {
            if e.is("w:p") || e.is("w:tbl") {
                let mut path = vec![i];
                objects(e, &mut path, i, &mut found);
            }
        }
    }
    found
        .into_iter()
        .enumerate()
        .filter_map(|(n, (anchor, targets))| {
            let mut cur = body;
            for i in &targets[0] {
                cur = cur.children.get(*i)?.as_element()?;
            }
            Some(TextBox { id: format!("tb{}", n + 1), anchor, paras: container_paras(cur, &targets[0], cx), targets })
        })
        .collect()
}

fn opaque_label(e: &Element) -> String {
    if e.is("w:sdt") {
        let pr = e.child("w:sdtPr");
        if let Some(g) = pr
            .and_then(|p| p.child_path(&["w:docPartObj", "w:docPartGallery"]))
            .and_then(|g| g.attr("w:val"))
        {
            return g.to_string();
        }
        if let Some(a) = pr.and_then(|p| p.child("w:alias")).and_then(|a| a.attr("w:val")) {
            return format!("content control: {a}");
        }
        return "content control".into();
    }
    e.local().to_string()
}

fn table(e: &Element, idx: usize, cx: &Ctx) -> Table {
    let mut rows = Vec::new();
    let mut simple = true;
    for (ri, rc) in e.children.iter().enumerate() {
        let Node::Element(tr) = rc else { continue };
        if !tr.is("w:tr") {
            if tr.is("w:sdt") || tr.is("w:customXml") {
                simple = false;
            }
            continue;
        }
        let mut cells = Vec::new();
        let mut cell_idx = Vec::new();
        for (ci, cc) in tr.children.iter().enumerate() {
            let Node::Element(tc) = cc else { continue };
            if !tc.is("w:tc") {
                if tc.is("w:sdt") || tc.is("w:customXml") {
                    simple = false;
                }
                continue;
            }
            if let Some(pr) = tc.child("w:tcPr") {
                let span = pr.child("w:gridSpan").and_then(|g| g.attr("w:val")).unwrap_or("1");
                if span != "1" || pr.child("w:vMerge").is_some() || pr.child("w:hMerge").is_some() {
                    simple = false;
                }
            }
            let mut paras = Vec::new();
            for (pi, pc) in tc.children.iter().enumerate() {
                let Node::Element(p) = pc else { continue };
                match p.name.as_str() {
                    "w:p" => paras.push(para(p, vec![idx, ri, ci, pi], cx)),
                    "w:tcPr" | "w:bookmarkStart" | "w:bookmarkEnd" | "w:proofErr" => {}
                    _ => simple = false,
                }
            }
            if paras.is_empty() {
                simple = false;
            }
            cells.push(Cell { paras });
            cell_idx.push(ci);
        }
        rows.push(Row { child_idx: ri, cells, cell_idx });
    }
    if let Some(n) = rows.first().map(|r| r.cells.len()) {
        if rows.iter().any(|r| r.cells.len() != n) || n == 0 {
            simple = false;
        }
    } else {
        simple = false;
    }
    Table { idx, rows, simple }
}

pub fn para(e: &Element, path: Vec<usize>, cx: &Ctx) -> Para {
    let ppr = e.child("w:pPr").cloned();
    let mut p = Para {
        path,
        shell: e.shell(),
        kind: cx.styles.para_kind(ppr.as_ref()),
        ppr,
        items: Vec::new(),
        runs: Vec::new(),
        wraps: Vec::new(),
    };
    let mut ctx = Vec::new();
    walk_content(&mut p, e, &mut ctx, cx);
    p
}

fn walk_content(p: &mut Para, parent: &Element, ctx: &mut Vec<usize>, cx: &Ctx) {
    for c in &parent.children {
        let Node::Element(e) = c else { continue };
        match e.name.as_str() {
            "w:pPr" => {}
            "w:r" => run(p, e, ctx, cx),
            "w:ins" | "w:del" | "w:moveTo" | "w:moveFrom" => {
                let kind = if matches!(e.name.as_str(), "w:ins" | "w:moveTo") { WrapKind::Ins } else { WrapKind::Del };
                push_wrap(p, e, ctx, cx, Wrap {
                    kind,
                    shell: e.shell(),
                    content_child: None,
                    author: e.attr("w:author").map(String::from),
                    link: None,
                });
            }
            "w:hyperlink" => {
                let link = e
                    .attr("r:id")
                    .and_then(|id| cx.rels.get(id))
                    .map(|r| r.target.to_string())
                    .or_else(|| e.attr("w:anchor").map(|a| format!("#{a}")))
                    .unwrap_or_default();
                push_wrap(p, e, ctx, cx, Wrap {
                    kind: WrapKind::Hyperlink,
                    shell: e.shell(),
                    content_child: None,
                    author: None,
                    link: Some(link),
                });
            }
            "w:smartTag" | "w:customXml" | "w:fldSimple" | "w:dir" | "w:bdo" => {
                // Keep property children (smartTagPr, customXmlPr) in the shell.
                let mut shell = e.shell();
                for pc in e.elements().filter(|c| c.name.ends_with("Pr")) {
                    shell.children.push(pc.clone().into());
                }
                push_wrap(p, e, ctx, cx, Wrap {
                    kind: WrapKind::Other,
                    shell,
                    content_child: None,
                    author: None,
                    link: None,
                });
            }
            "w:sdt" => {
                let mut shell = e.shell();
                for pc in e.elements().filter(|c| !c.is("w:sdtContent")) {
                    shell.children.push(pc.clone().into());
                }
                let content = e.child("w:sdtContent");
                let w = p.wraps.len();
                p.wraps.push(Wrap {
                    kind: WrapKind::Other,
                    shell,
                    content_child: Some(content.map(|c| c.shell()).unwrap_or_else(|| Element::new("w:sdtContent"))),
                    author: None,
                    link: None,
                });
                if let Some(content) = content {
                    ctx.push(w);
                    walk_content(p, content, ctx, cx);
                    ctx.pop();
                }
            }
            "m:oMath" | "m:oMathPara" => p.items.push(Item {
                ctx: ctx.clone(),
                run: None,
                what: What::Obj { key: "<!--docex:math-->".into(), node: c.clone() },
            }),
            "mc:AlternateContent" => p.items.push(Item {
                ctx: ctx.clone(),
                run: None,
                what: What::Obj { key: "<!--docex:shape-->".into(), node: c.clone() },
            }),
            _ => p.items.push(Item { ctx: ctx.clone(), run: None, what: What::Hidden(c.clone()) }),
        }
    }
}

fn push_wrap(p: &mut Para, e: &Element, ctx: &mut Vec<usize>, cx: &Ctx, w: Wrap) {
    let id = p.wraps.len();
    p.wraps.push(w);
    ctx.push(id);
    walk_content(p, e, ctx, cx);
    ctx.pop();
}

fn run(p: &mut Para, r: &Element, ctx: &[usize], cx: &Ctx) {
    let rpr = r.child("w:rPr").cloned();
    let marks = marks_of(rpr.as_ref(), cx.styles);
    let ri = p.runs.len();
    p.runs.push(RunInfo { shell: r.shell(), rpr, marks });
    for c in &r.children {
        let Node::Element(e) = c else { continue };
        let obj = |key: &str| What::Obj { key: key.to_string(), node: c.clone() };
        let what = match e.name.as_str() {
            "w:rPr" => continue,
            "w:t" | "w:delText" => {
                for ch in e.text_content().chars() {
                    p.items.push(Item { ctx: ctx.to_vec(), run: Some(ri), what: What::Char(ch) });
                }
                continue;
            }
            "w:tab" | "w:ptab" => obj("\t"),
            "w:br" | "w:cr" => match e.attr("w:type") {
                Some("page") => obj("<!--docex:pagebreak-->"),
                Some("column") => obj("<!--docex:columnbreak-->"),
                _ => obj("\n"),
            },
            "w:noBreakHyphen" => obj("-"),
            "w:sym" => obj("<!--docex:sym-->"),
            "w:footnoteReference" => obj(&format!("fn:{}", e.attr("w:id").unwrap_or("?"))),
            "w:endnoteReference" => obj(&format!("fn:e{}", e.attr("w:id").unwrap_or("?"))),
            "w:drawing" | "w:pict" | "w:object" => match image_target(e, cx.rels) {
                Some(t) => obj(&format!("img:{t}")),
                None if e.is("w:drawing") => obj("<!--docex:drawing-->"),
                None => obj("<!--docex:object-->"),
            },
            "mc:AlternateContent" => obj("<!--docex:shape-->"),
            _ => What::Hidden(c.clone()),
        };
        p.items.push(Item { ctx: ctx.to_vec(), run: Some(ri), what });
    }
}

/// Media target (relative to `word/`) of an inline picture, if it is one.
fn image_target(e: &Element, rels: &Rels) -> Option<String> {
    let mut found = None;
    e.visit(&mut |n| {
        if found.is_none() && (n.is("a:blip") || n.is("v:imagedata")) {
            if let Some(id) = n.attr("r:embed").or(n.attr("r:id")) {
                found = rels.get(id).map(|r| r.target.to_string());
            }
        }
    });
    // Charts, SmartArt and text boxes are not pictures.
    let mut is_textbox = false;
    e.visit(&mut |n| {
        if n.is("wps:txbx") || n.is("c:chart") || n.is("dgm:relIds") {
            is_textbox = true;
        }
    });
    if is_textbox { None } else { found }
}

/// Alt text of a drawing.
pub fn image_alt(node: &Node) -> String {
    let mut alt = String::new();
    if let Node::Element(e) = node {
        e.visit(&mut |n| {
            if alt.is_empty() && n.is("wp:docPr") {
                alt = n.attr("descr").unwrap_or("").to_string();
            }
        });
    }
    alt
}
