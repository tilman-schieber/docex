//! Markdown + CriticMarkup -> blocks of atoms, the comparable form of both
//! the rendered original and the user's edited text.

use crate::model::{Marks, Status};
use crate::styles::Kind;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EAtom {
    pub key: String,
    pub marks: Marks,
    pub link: Option<String>,
    pub st: Status,
}

/// A comment written into the Markdown: `{==anchor==}{>>text<<}` or `{>>text<<}`.
#[derive(Clone, Debug)]
pub struct EComment {
    /// Atom range the comment is anchored to (empty range = point comment).
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct EPara {
    pub kind: Kind,
    pub atoms: Vec<EAtom>,
    pub comments: Vec<EComment>,
}

#[derive(Clone, Debug)]
pub enum EBlock {
    Para(EPara),
    /// rows -> cells -> paragraphs
    Table(Vec<Vec<Vec<EPara>>>),
    Opaque(usize),
}

impl EPara {
    pub fn text(&self) -> String {
        self.atoms.iter().map(|a| a.key.as_str()).collect()
    }
}

impl EBlock {
    /// Exact-identity key for block alignment.
    pub fn key(&self) -> String {
        match self {
            EBlock::Para(p) => {
                let cm: Vec<String> = p.comments.iter().map(|c| format!("{}-{}:{}", c.start, c.end, c.text)).collect();
                format!("P{:?}|{}|{}", p.kind, atoms_key(&p.atoms), cm.join("\u{1}"))
            }
            EBlock::Table(rows) => {
                let mut s = String::from("T");
                for r in rows {
                    s.push('\u{1}');
                    for c in r {
                        s.push('\u{2}');
                        for p in c {
                            s.push('\u{3}');
                            s.push_str(&atoms_key(&p.atoms));
                        }
                    }
                }
                s
            }
            EBlock::Opaque(n) => format!("O{n}"),
        }
    }

    /// Visible text for similarity scoring.
    pub fn text(&self) -> String {
        match self {
            EBlock::Para(p) => p.text(),
            EBlock::Table(rows) => rows
                .iter()
                .flat_map(|r| r.iter().flat_map(|c| c.iter().map(|p| p.text())))
                .collect::<Vec<_>>()
                .join(" "),
            EBlock::Opaque(n) => format!("\u{0}opaque{n}"),
        }
    }
}

pub fn atoms_key(atoms: &[EAtom]) -> String {
    let mut s = String::new();
    for a in atoms {
        s.push_str(&a.key);
        s.push(match a.st {
            Status::Plain => '\u{4}',
            Status::Ins => '\u{5}',
            Status::Del => '\u{6}',
        });
        s.push(char::from(b'a' + (a.marks.bold as u8) + 2 * (a.marks.italic as u8) + 4 * (a.marks.strike as u8) + 8 * (a.marks.code as u8)));
        if let Some(l) = &a.link {
            s.push_str(l);
        }
    }
    s
}

// Sentinels for CriticMarkup delimiters, substituted before Markdown parsing.
const INS_O: char = '\u{FDD0}';
const INS_C: char = '\u{FDD1}';
const DEL_O: char = '\u{FDD2}';
const DEL_C: char = '\u{FDD3}';
const SUB_O: char = '\u{FDD4}';
const SUB_M: char = '\u{FDD5}';
const SUB_C: char = '\u{FDD6}';
const HL_O: char = '\u{FDD7}';
const HL_C: char = '\u{FDD8}';
const CM_O: char = '\u{FDD9}';
const CM_C: char = '\u{FDDA}';

fn is_sentinel(c: char) -> bool {
    ('\u{FDD0}'..='\u{FDDA}').contains(&c)
}

/// Replace CriticMarkup delimiters with sentinel characters.
///
/// Each kind is tracked on its own rather than as a stack, because ranges
/// from Word overlap: a comment can start inside an insertion and end after
/// it (`{++a {==b++} c==}`).
fn critic_prepass(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let (mut ins, mut del, mut sub, mut hl, mut cm) = (0u32, 0u32, 0u32, 0u32, false);
    let mut it = src.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let rest = &src[i..];
        if c == '\\' {
            out.push(c);
            if let Some((_, n)) = it.next() {
                out.push(n);
            }
            continue;
        }
        // Review id markers from `docex cat --ids` (`⟦c12⟧`) are not text.
        if c == '⟦' {
            if let Some(end) = rest.find('⟧') {
                let inner = &rest[c.len_utf8()..end];
                let mut ch = inner.chars();
                if matches!(ch.next(), Some('c' | 'f' | 'e' | 'm')) && !ch.as_str().is_empty() && ch.as_str().chars().all(|d| d.is_ascii_digit()) {
                    while let Some((j, _)) = it.peek() {
                        if *j >= i + end + '⟧'.len_utf8() {
                            break;
                        }
                        it.next();
                    }
                    continue;
                }
            }
        }
        let mut take = |tok: &str, sent: char, out: &mut String| {
            out.push(sent);
            for _ in 1..tok.chars().count() {
                it.next();
            }
        };
        if cm {
            if rest.starts_with("<<}") {
                take("<<}", CM_C, &mut out);
                cm = false;
            } else {
                out.push(c);
            }
            continue;
        }
        let tok = [
            ("{++", INS_O),
            ("{--", DEL_O),
            ("{~~", SUB_O),
            ("{==", HL_O),
            ("{>>", CM_O),
            ("++}", INS_C),
            ("--}", DEL_C),
            ("~>", SUB_M),
            ("~~}", SUB_C),
            ("==}", HL_C),
        ]
        .into_iter()
        .find(|(t, sent)| {
            rest.starts_with(t)
                && match *sent {
                    INS_C => ins > 0,
                    DEL_C => del > 0,
                    SUB_M | SUB_C => sub > 0,
                    HL_C => hl > 0,
                    _ => true,
                }
        });
        match tok {
            Some((t, sent)) => {
                take(t, sent, &mut out);
                match sent {
                    INS_O => ins += 1,
                    DEL_O => del += 1,
                    SUB_O => sub += 1,
                    HL_O => hl += 1,
                    CM_O => cm = true,
                    INS_C => ins -= 1,
                    DEL_C => del -= 1,
                    SUB_C => sub -= 1,
                    HL_C => hl -= 1,
                    _ => {}
                }
            }
            None => out.push(c),
        }
    }
    out
}

#[derive(Default)]
struct Inline {
    bold: u32,
    italic: u32,
    strike: u32,
    link: Vec<String>,
    st: Vec<Status>,
    sub_second: bool,
    in_comment: Option<String>,
    hl_start: Vec<usize>,
    image: Option<(String, String)>,
}

struct P {
    blocks: Vec<EBlock>,
    para: Option<EPara>,
    inl: Inline,
    lists: Vec<bool>,
    quote: u32,
    heading: Option<u8>,
    code: Option<(String, String)>,
    html: Option<String>,
    /// Footnote definitions: label -> paragraphs.
    notes: Vec<(String, Vec<EPara>)>,
    in_note: bool,
    /// Text box sections: id -> paragraphs.
    boxes: Vec<(String, Vec<EPara>)>,
    in_box: bool,
    table: Option<Vec<Vec<Vec<EPara>>>>,
    row: Option<Vec<Vec<EPara>>>,
    cell: Option<Vec<EPara>>,
}

fn new_para(kind: Kind) -> EPara {
    EPara { kind, atoms: Vec::new(), comments: Vec::new() }
}

impl P {
    fn context_kind(&self) -> Kind {
        if let Some(h) = self.heading {
            return Kind::Heading(h);
        }
        if let Some(ordered) = self.lists.last() {
            let lvl = (self.lists.len() - 1) as u8;
            return if *ordered { Kind::Number(lvl) } else { Kind::Bullet(lvl) };
        }
        if self.quote > 0 {
            return Kind::Quote;
        }
        Kind::Normal
    }

    fn target(&mut self) -> Option<&mut EPara> {
        if self.cell.is_some() {
            let cell = self.cell.as_mut().unwrap();
            if cell.is_empty() {
                cell.push(new_para(Kind::Normal));
            }
            return cell.last_mut();
        }
        if self.para.is_none() {
            let k = self.context_kind();
            self.para = Some(new_para(k));
        }
        self.para.as_mut()
    }

    fn close_para(&mut self) {
        if let Some(mut p) = self.para.take() {
            if p.kind == Kind::Caption && p.atoms.first().is_some_and(|a| a.key == ":") {
                p.atoms.remove(0);
                for c in &mut p.comments {
                    c.start = c.start.saturating_sub(1);
                    c.end = c.end.saturating_sub(1);
                }
            }
            trim(&mut p);
            if self.in_box {
                p.kind = Kind::Normal;
                if let Some((_, paras)) = self.boxes.last_mut() {
                    if !p.atoms.is_empty() || !p.comments.is_empty() {
                        paras.push(p);
                    }
                }
                return;
            }
            if self.in_note {
                p.kind = Kind::Normal;
                if let Some((_, paras)) = self.notes.last_mut() {
                    if !p.atoms.is_empty() || !p.comments.is_empty() {
                        paras.push(p);
                    }
                }
                return;
            }
            if !p.atoms.is_empty() || !p.comments.is_empty() {
                self.blocks.push(EBlock::Para(p));
            }
        }
    }

    fn marks(&self, code: bool) -> Marks {
        Marks { bold: self.inl.bold > 0, italic: self.inl.italic > 0, strike: self.inl.strike > 0, code }
    }

    fn push_text(&mut self, s: &str, code: bool) {
        for c in s.chars() {
            if is_sentinel(c) {
                self.sentinel(c);
                continue;
            }
            if let Some(cm) = &mut self.inl.in_comment {
                cm.push(c);
                continue;
            }
            if let Some((_, alt)) = &mut self.inl.image {
                alt.push(c);
                continue;
            }
            let st = self.status();
            if st == Status::Del && self.inl.sub_second {
                continue;
            }
            let atom = EAtom {
                key: c.to_string(),
                marks: self.marks(code),
                link: self.inl.link.last().cloned(),
                st,
            };
            if let Some(p) = self.target() {
                p.atoms.push(atom);
            }
        }
    }

    fn push_obj(&mut self, key: String) {
        if self.inl.in_comment.is_some() {
            return;
        }
        let atom = EAtom { key, marks: self.marks(false), link: self.inl.link.last().cloned(), st: self.status() };
        if let Some(p) = self.target() {
            p.atoms.push(atom);
        }
    }

    fn status(&self) -> Status {
        match self.inl.st.last() {
            Some(Status::Del) if self.inl.sub_second => Status::Ins,
            Some(s) => *s,
            None => Status::Plain,
        }
    }

    fn sentinel(&mut self, c: char) {
        match c {
            INS_O => self.inl.st.push(Status::Ins),
            DEL_O => self.inl.st.push(Status::Del),
            SUB_O => {
                self.inl.st.push(Status::Del);
                self.inl.sub_second = false;
            }
            SUB_M => self.inl.sub_second = true,
            INS_C | DEL_C | SUB_C => {
                self.inl.st.pop();
                self.inl.sub_second = false;
            }
            HL_O => {
                let n = self.target().map(|p| p.atoms.len()).unwrap_or(0);
                self.inl.hl_start.push(n);
            }
            HL_C => {}
            CM_O => self.inl.in_comment = Some(String::new()),
            CM_C => {
                let text = self.inl.in_comment.take().unwrap_or_default();
                let start = self.inl.hl_start.pop();
                if let Some(p) = self.target() {
                    let end = p.atoms.len();
                    p.comments.push(EComment { start: start.unwrap_or(end), end, text: text.trim().to_string() });
                }
            }
            _ => {}
        }
    }
}

fn trim(p: &mut EPara) {
    let ws = |a: &EAtom| a.key.chars().all(char::is_whitespace);
    while p.atoms.last().is_some_and(ws) {
        p.atoms.pop();
    }
    let lead = p.atoms.iter().take_while(|a| ws(a)).count();
    if lead > 0 {
        p.atoms.drain(..lead);
        for c in &mut p.comments {
            c.start = c.start.saturating_sub(lead);
            c.end = c.end.saturating_sub(lead);
        }
    }
    let n = p.atoms.len();
    for c in &mut p.comments {
        c.start = c.start.min(n);
        c.end = c.end.min(n);
    }
}

fn opaque_id(s: &str, prefix: &str) -> Option<usize> {
    let i = s.find(prefix)? + prefix.len();
    let digits: String = s[i..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Blocks plus footnote definitions (`[^label]: text`).
pub struct Parsed {
    pub blocks: Vec<EBlock>,
    pub notes: Vec<(String, Vec<EPara>)>,
    pub boxes: Vec<(String, Vec<EPara>)>,
}

pub fn parse(src: &str) -> Vec<EBlock> {
    parse_full(src).blocks
}

pub fn parse_full(src: &str) -> Parsed {
    let pre = critic_prepass(src);
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_FOOTNOTES | Options::ENABLE_MATH;
    let mut p = P {
        blocks: Vec::new(),
        para: None,
        inl: Inline::default(),
        lists: Vec::new(),
        quote: 0,
        heading: None,
        code: None,
        html: None,
        notes: Vec::new(),
        in_note: false,
        boxes: Vec::new(),
        in_box: false,
        table: None,
        row: None,
        cell: None,
    };
    for (ev, range) in Parser::new_ext(&pre, opts).into_offset_iter() {
        if let Some((_, buf)) = &mut p.code {
            match ev {
                Event::Text(t) => buf.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (info, buf) = p.code.take().unwrap();
                    if info.starts_with("docex-table") {
                        if let Some(n) = opaque_id(&info, " b") {
                            p.blocks.push(EBlock::Opaque(n));
                        }
                        continue;
                    }
                    if info.trim() == "br" {
                        // One paragraph whose lines are line breaks.
                        p.para = Some(new_para(Kind::Code));
                        for (i, line) in buf.trim_end_matches('\n').split('\n').enumerate() {
                            if i > 0 {
                                p.push_obj("\n".into());
                            }
                            p.push_text(line, false);
                        }
                        p.close_para();
                        p.inl = Inline::default();
                        continue;
                    }
                    // One code paragraph per line; CriticMarkup still applies.
                    for line in buf.lines() {
                        p.para = Some(new_para(Kind::Code));
                        for (i, part) in line.split("<br/>").enumerate() {
                            if i > 0 {
                                p.push_obj("\n".into());
                            }
                            p.push_text(part, false);
                        }
                        p.close_para();
                    }
                    p.inl = Inline::default();
                }
                _ => {}
            }
            continue;
        }
        if let Some(buf) = &mut p.html {
            match ev {
                Event::Html(t) | Event::Text(t) => buf.push_str(&t),
                Event::End(TagEnd::HtmlBlock) => {
                    let buf = p.html.take().unwrap();
                    if buf.contains(crate::render::BOX_END) {
                        p.close_para();
                        p.in_box = false;
                    } else if let Some(i) = buf.find(crate::render::BOX_START) {
                        p.close_para();
                        let id: String = buf[i + crate::render::BOX_START.len()..]
                            .trim_start()
                            .chars()
                            .take_while(|c| c.is_alphanumeric())
                            .collect();
                        p.boxes.push((id, Vec::new()));
                        p.in_box = true;
                    } else if let Some(n) = opaque_id(&buf, "docex:opaque b") {
                        p.blocks.push(EBlock::Opaque(n));
                    } else if buf.contains("<!--docex:") {
                        // A paragraph that merely starts with an inline object
                        // (`<!--docex:shape-->`): read it as inline text.
                        let inner = parse(&format!("x{}", buf.trim_end()));
                        if let Some(EBlock::Para(mut ip)) = inner.into_iter().next() {
                            ip.atoms.remove(0);
                            for c in &mut ip.comments {
                                c.start = c.start.saturating_sub(1);
                                c.end = c.end.saturating_sub(1);
                            }
                            ip.kind = p.context_kind();
                            p.close_para();
                            trim(&mut ip);
                            if !ip.atoms.is_empty() {
                                p.blocks.push(EBlock::Para(ip));
                            }
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if p.cell.is_none() {
                        p.close_para();
                        // `: text` is a caption (an escaped `\:` is not).
                        if !p.in_note && !p.in_box && p.context_kind() == Kind::Normal && pre[range.start..].starts_with(": ") {
                            p.para = Some(new_para(Kind::Caption));
                        }
                    }
                }
                Tag::Heading { level, .. } => {
                    p.close_para();
                    p.heading = Some(level as u8);
                }
                Tag::BlockQuote(_) => {
                    p.close_para();
                    p.quote += 1;
                }
                Tag::CodeBlock(kind) => {
                    p.close_para();
                    let info = match kind {
                        CodeBlockKind::Fenced(i) => i.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    p.code = Some((info, String::new()));
                }
                Tag::HtmlBlock => {
                    p.close_para();
                    p.html = Some(String::new());
                }
                Tag::List(start) => {
                    p.close_para();
                    p.lists.push(start.is_some());
                }
                Tag::Item => p.close_para(),
                Tag::FootnoteDefinition(label) => {
                    p.close_para();
                    p.notes.push((label.to_string(), Vec::new()));
                    p.in_note = true;
                }
                Tag::Table(_) => {
                    p.close_para();
                    p.table = Some(Vec::new());
                }
                Tag::TableHead | Tag::TableRow => p.row = Some(Vec::new()),
                Tag::TableCell => p.cell = Some(Vec::new()),
                Tag::Emphasis => p.inl.italic += 1,
                Tag::Strong => p.inl.bold += 1,
                Tag::Strikethrough => p.inl.strike += 1,
                Tag::Link { dest_url, .. } => p.inl.link.push(dest_url.to_string()),
                Tag::Image { dest_url, .. } => p.inl.image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph => {
                    if p.cell.is_none() {
                        p.close_para();
                    }
                }
                TagEnd::Heading(_) => {
                    p.close_para();
                    p.heading = None;
                }
                TagEnd::BlockQuote(_) => {
                    p.close_para();
                    p.quote = p.quote.saturating_sub(1);
                }
                TagEnd::List(_) => {
                    p.close_para();
                    p.lists.pop();
                }
                TagEnd::Item => p.close_para(),
                TagEnd::FootnoteDefinition => {
                    p.close_para();
                    p.in_note = false;
                }
                TagEnd::TableCell => {
                    let mut cell = p.cell.take().unwrap_or_default();
                    if cell.is_empty() {
                        cell.push(new_para(Kind::Normal));
                    }
                    for para in &mut cell {
                        trim(para);
                    }
                    if let Some(r) = &mut p.row {
                        r.push(cell);
                    }
                }
                TagEnd::TableHead | TagEnd::TableRow => {
                    if let (Some(t), Some(r)) = (&mut p.table, p.row.take()) {
                        t.push(r);
                    }
                }
                TagEnd::Table => {
                    if let Some(t) = p.table.take() {
                        p.blocks.push(EBlock::Table(t));
                    }
                }
                TagEnd::Emphasis => p.inl.italic = p.inl.italic.saturating_sub(1),
                TagEnd::Strong => p.inl.bold = p.inl.bold.saturating_sub(1),
                TagEnd::Strikethrough => p.inl.strike = p.inl.strike.saturating_sub(1),
                TagEnd::Link => {
                    p.inl.link.pop();
                }
                TagEnd::Image => {
                    if let Some((url, _alt)) = p.inl.image.take() {
                        p.push_obj(format!("img:{url}"));
                    }
                }
                _ => {}
            },
            Event::Text(t) => p.push_text(&t, false),
            Event::Code(t) => p.push_text(&t, true),
            Event::SoftBreak => p.push_text(" ", false),
            Event::HardBreak => p.push_obj("\n".into()),
            Event::FootnoteReference(l) => p.push_obj(format!("fn:{l}")),
            Event::InlineMath(m) => p.push_obj(format!("math:{}", m.trim())),
            Event::DisplayMath(m) => p.push_obj(format!("mathd:{}", m.split_whitespace().collect::<Vec<_>>().join(" "))),
            Event::InlineHtml(h) => {
                let t = h.trim();
                let lower = t.to_ascii_lowercase();
                if lower == "<br>" && p.cell.is_some() {
                    if let Some(c) = &mut p.cell {
                        if c.is_empty() {
                            c.push(new_para(Kind::Normal));
                        }
                        c.push(new_para(Kind::Normal));
                    }
                } else if lower == "<br>" || lower == "<br/>" || lower == "<br />" {
                    p.push_obj("\n".into());
                } else if t.starts_with("<!--docex:") {
                    p.push_obj(t.to_string());
                } else {
                    p.push_text(t, false);
                }
            }
            Event::Html(h) => p.push_text(&h, false),
            Event::TaskListMarker(done) => p.push_text(if done { "[x] " } else { "[ ] " }, false),
            _ => {}
        }
    }
    p.close_para();
    Parsed { blocks: p.blocks, notes: p.notes, boxes: p.boxes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(b: &EBlock) -> String {
        b.text()
    }

    #[test]
    fn critic() {
        let b = parse("Hello {++big ++}world{--old--} and {~~a~>b~~}.");
        let EBlock::Para(p) = &b[0] else { panic!() };
        let st: String = p
            .atoms
            .iter()
            .map(|a| match a.st {
                Status::Plain => '.',
                Status::Ins => '+',
                Status::Del => '-',
            })
            .collect();
        assert_eq!(text(&b[0]), "Hello big worldold and ab.");
        assert_eq!(st, "......++++.....---.....-+.");
    }

    #[test]
    fn blocks() {
        let b = parse("# T\n\n- a\n  - b\n1. c\n\n> q\n\n| x | y |\n| --- | --- |\n| 1<br>2 | **3** |\n");
        let kinds: Vec<String> = b
            .iter()
            .map(|b| match b {
                EBlock::Para(p) => format!("{:?}", p.kind),
                EBlock::Table(r) => format!("T{}x{}:{}", r.len(), r[1].len(), r[1][0].len()),
                EBlock::Opaque(n) => format!("O{n}"),
            })
            .collect();
        assert_eq!(kinds, ["Heading(1)", "Bullet(0)", "Bullet(1)", "Number(0)", "Quote", "T2x2:2"]);
    }

    #[test]
    fn overlapping_comment_and_insertion() {
        let b = parse("x {++Die {==FHNW++} muss==}{>>A: why<<} y");
        let EBlock::Para(p) = &b[0] else { panic!() };
        assert_eq!(p.text(), "x Die FHNW muss y");
        let ins: String = p.atoms.iter().filter(|a| a.st == Status::Ins).map(|a| a.key.as_str()).collect();
        assert_eq!(ins, "Die FHNW");
        assert_eq!((p.comments[0].start, p.comments[0].end), (6, 15));
    }

    #[test]
    fn comments() {
        let b = parse("See {==this==}{>>why?<<} now");
        let EBlock::Para(p) = &b[0] else { panic!() };
        assert_eq!(p.text(), "See this now");
        assert_eq!((p.comments[0].start, p.comments[0].end, p.comments[0].text.as_str()), (4, 8, "why?"));
    }
}
