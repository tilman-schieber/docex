//! Projection -> Markdown with CriticMarkup.

use crate::comments::Comments;
use crate::model::{image_alt, Block, Marks, Para, Status, Table, What};
use crate::styles::Kind;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// Tracked changes and comments as CriticMarkup (the editable view).
    Review,
    /// As if all changes were accepted.
    Accepted,
    /// As if all changes were rejected.
    Original,
}

pub struct RenderCtx<'a> {
    pub view: View,
    pub comments: &'a Comments,
    /// Comment ids that have a `commentRangeEnd` somewhere in the document.
    pub ranged: HashSet<String>,
    pub notes: &'a [(String, String)],
}

pub const HEADER: &str = "<!-- docex: edit freely. {++insert++} {--delete--} {~~old~>new~~} {>>comment<<} -->";

#[derive(Clone, Copy, Default)]
struct Mode {
    table: bool,
    heading: bool,
    code: bool,
}

enum Tok {
    Atom { key: String, st: Status, link: Option<String>, marks: Marks, alt: String },
    Marker(String),
}

/// Render blocks. Returns the Markdown and, for every Markdown block emitted,
/// the index of the projection block it came from.
pub fn render(blocks: &[Block], cx: &RenderCtx) -> (String, Vec<usize>) {
    let mut out = String::new();
    let mut map = Vec::new();
    if cx.view == View::Review {
        out.push_str(HEADER);
    }
    let mut prev_list = false;
    let mut prev_top: Option<bool> = None;
    let mut list_stack: Vec<usize> = Vec::new();
    let mut in_code = false;
    let close_code = |out: &mut String, in_code: &mut bool| {
        if *in_code {
            out.push_str("\n````");
            *in_code = false;
        }
    };
    for (bi, b) in blocks.iter().enumerate() {
        match b {
            Block::Para(p) => {
                let mode = Mode { heading: matches!(p.kind, Kind::Heading(_)), code: p.kind == Kind::Code, table: false };
                let Some(text) = inline(p, mode, cx) else { continue };
                if p.kind == Kind::Code {
                    // A code paragraph with line breaks gets a fence of its own
                    // ("br"), where each line is a break, not a paragraph.
                    let has_br = text.contains("<br/>");
                    if has_br || !in_code {
                        close_code(&mut out, &mut in_code);
                        sep(&mut out, false);
                        out.push_str(if has_br { "````br" } else { "````" });
                        in_code = true;
                    }
                    out.push('\n');
                    out.push_str(&text.replace("<br/>", "\n"));
                    if has_br {
                        close_code(&mut out, &mut in_code);
                    }
                    map.push(bi);
                    prev_list = false;
                    continue;
                }
                close_code(&mut out, &mut in_code);
                let is_list = p.kind.is_list();
                // Adjacent lists of different types need a blank line between them.
                let top_marker = match p.kind {
                    Kind::Bullet(0) => Some(true),
                    Kind::Number(0) => Some(false),
                    _ => None,
                };
                let same_list = top_marker.is_none_or(|m| prev_top == Some(m));
                sep(&mut out, prev_list && is_list && same_list);
                if !is_list || !same_list {
                    list_stack.clear();
                }
                if let Some(m) = top_marker {
                    prev_top = Some(m);
                }
                match p.kind {
                    Kind::Heading(n) => {
                        out.push_str(&"#".repeat(n as usize));
                        out.push(' ');
                    }
                    Kind::Quote => out.push_str("> "),
                    Kind::Bullet(l) | Kind::Number(l) => {
                        let l = (l as usize).min(list_stack.len());
                        list_stack.truncate(l);
                        let indent = if l == 0 { 0 } else { list_stack[l - 1] };
                        let marker = if matches!(p.kind, Kind::Bullet(_)) { "- " } else { "1. " };
                        out.push_str(&" ".repeat(indent));
                        out.push_str(marker);
                        list_stack.push(indent + marker.len());
                    }
                    _ => {}
                }
                out.push_str(&text);
                map.push(bi);
                prev_list = is_list;
                if !is_list {
                    prev_top = None;
                }
            }
            Block::Table(t) => {
                close_code(&mut out, &mut in_code);
                sep(&mut out, false);
                list_stack.clear();
                prev_list = false;
                if t.simple {
                    out.push_str(&pipe_table(t, cx));
                } else {
                    out.push_str(&grid_table(t, cx));
                }
                map.push(bi);
            }
            Block::Opaque { idx, label } => {
                close_code(&mut out, &mut in_code);
                sep(&mut out, false);
                list_stack.clear();
                prev_list = false;
                out.push_str(&format!("<!-- docex:opaque b{idx} {} -->", label.replace("--", "-")));
                map.push(bi);
            }
        }
    }
    close_code(&mut out, &mut in_code);
    let notes: Vec<&(String, String)> = cx.notes.iter().filter(|(id, _)| out.contains(&format!("[^{id}]"))).collect();
    for (id, text) in notes {
        sep(&mut out, false);
        out.push_str(&format!("[^{id}]: {}", escape_plain(&text.replace('\n', " "))));
    }
    out.push('\n');
    (out, map)
}

fn sep(out: &mut String, tight: bool) {
    if out.is_empty() {
        return;
    }
    out.push_str(if tight { "\n" } else { "\n\n" });
}

fn pipe_table(t: &Table, cx: &RenderCtx) -> String {
    let ncols = t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    let mut s = String::new();
    for (ri, row) in t.rows.iter().enumerate() {
        s.push('|');
        for ci in 0..ncols {
            let cell = row
                .cells
                .get(ci)
                .map(|c| {
                    c.paras
                        .iter()
                        .map(|p| inline(p, Mode { table: true, ..Default::default() }, cx).unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("<br>")
                })
                .unwrap_or_default();
            s.push(' ');
            s.push_str(&cell);
            s.push_str(" |");
        }
        if ri == 0 {
            s.push_str("\n|");
            s.push_str(&" --- |".repeat(ncols));
        }
        if ri + 1 < t.rows.len() {
            s.push('\n');
        }
    }
    s
}

fn grid_table(t: &Table, cx: &RenderCtx) -> String {
    let mut s = format!("````docex-table b{} (merged cells: read-only)\n", t.idx);
    for row in &t.rows {
        let cells: Vec<String> = row
            .cells
            .iter()
            .map(|c| c.paras.iter().map(|p| view_text(p, cx.view)).collect::<Vec<_>>().join(" / "))
            .collect();
        s.push_str(&cells.join(" | "));
        s.push('\n');
    }
    s.push_str("````");
    s
}

/// Plain text of a paragraph in a view (no markup).
pub fn view_text(p: &Para, view: View) -> String {
    p.visible()
        .filter(|(_, i)| keep_in_view(p.status(i), view))
        .filter_map(|(_, i)| match &i.what {
            What::Char(c) => Some(c.to_string()),
            What::Obj { key, .. } if key == "\t" => Some(" ".into()),
            What::Obj { key, .. } if key == "\n" => Some(" ".into()),
            _ => None,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn keep_in_view(st: Status, view: View) -> bool {
    !matches!((view, st), (View::Accepted, Status::Del) | (View::Original, Status::Ins))
}

fn comment_markers(id: &str, cx: &RenderCtx) -> String {
    let mut s = String::new();
    let Some(c) = cx.comments.get(id) else { return s };
    let mut one = |c: &crate::comments::Comment, reply: bool| {
        let text = escape_plain(&c.text.replace('\n', " ")).replace("<<}", "<< }");
        s.push_str(&format!(
            "{{>>{}{}{}: {}<<}}",
            if reply { "↳ " } else { "" },
            if c.done { "✓ " } else { "" },
            escape_plain(&c.author),
            text
        ));
    };
    one(c, false);
    for r in cx.comments.replies(id) {
        one(r, true);
    }
    s
}

/// Inline Markdown for one paragraph, `None` if nothing visible remains.
fn inline(p: &Para, mode: Mode, cx: &RenderCtx) -> Option<String> {
    let mut toks = Vec::new();
    let review = cx.view == View::Review;
    // Comment anchors: only pair {== ==} when both ends are in this paragraph.
    let mut starts = HashSet::new();
    let mut ends = HashSet::new();
    for it in &p.items {
        if let What::Hidden(crate::xml::Node::Element(e)) = &it.what {
            let id = e.attr("w:id").unwrap_or("").to_string();
            match e.name.as_str() {
                "w:commentRangeStart" => {
                    starts.insert(id);
                }
                "w:commentRangeEnd" => {
                    ends.insert(id);
                }
                _ => {}
            }
        }
    }
    for it in &p.items {
        match &it.what {
            What::Hidden(crate::xml::Node::Element(e)) => {
                if !review {
                    continue;
                }
                let id = e.attr("w:id").unwrap_or("");
                if cx.comments.get(id).is_none() {
                    continue;
                }
                match e.name.as_str() {
                    "w:commentRangeStart" if ends.contains(id) => toks.push(Tok::Marker("{==".into())),
                    "w:commentRangeEnd" => {
                        let mut m = String::new();
                        if starts.contains(id) {
                            m.push_str("==}");
                        }
                        m.push_str(&comment_markers(id, cx));
                        toks.push(Tok::Marker(m));
                    }
                    "w:commentReference" if !cx.ranged.contains(id) => {
                        toks.push(Tok::Marker(comment_markers(id, cx)))
                    }
                    _ => {}
                }
            }
            What::Hidden(_) => {}
            What::Char(_) | What::Obj { .. } => {
                let st = p.status(it);
                if !keep_in_view(st, cx.view) {
                    continue;
                }
                let alt = match &it.what {
                    What::Obj { key, node } if key.starts_with("img:") => image_alt(node),
                    _ => String::new(),
                };
                toks.push(Tok::Atom {
                    key: it.key().unwrap(),
                    st: if review { st } else { Status::Plain },
                    link: if mode.code { None } else { p.link(it).map(String::from) },
                    marks: if mode.code { Marks::default() } else { p.marks(it) },
                    alt,
                });
            }
        }
    }
    // Trim whitespace atoms at both ends.
    let is_ws = |t: &Tok| matches!(t, Tok::Atom { key, .. } if key.chars().all(char::is_whitespace));
    let first = toks.iter().position(|t| matches!(t, Tok::Atom { .. }) && !is_ws(t))?;
    let last = toks.iter().rposition(|t| matches!(t, Tok::Atom { .. }) && !is_ws(t))?;
    let toks: Vec<Tok> = toks
        .into_iter()
        .enumerate()
        .filter(|(i, t)| (*i >= first && *i <= last) || matches!(t, Tok::Marker(_)))
        .map(|(_, t)| t)
        .collect();
    let mut w = Writer { out: String::new(), mode, st: Status::Plain, link: None };
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Marker(m) => {
                w.out.push_str(m);
                i += 1;
            }
            Tok::Atom { st, link, marks, .. } => {
                let mut j = i;
                let mut seg = Vec::new();
                while let Some(Tok::Atom { key, st: s2, link: l2, marks: m2, alt }) = toks.get(j) {
                    if s2 != st || l2 != link || m2 != marks {
                        break;
                    }
                    seg.push((key.as_str(), alt.as_str()));
                    j += 1;
                }
                w.segment(*st, link.clone(), *marks, &seg);
                i = j;
            }
        }
    }
    w.set_link(None);
    w.set_status(Status::Plain);
    Some(w.out)
}

struct Writer {
    out: String,
    mode: Mode,
    st: Status,
    link: Option<String>,
}

impl Writer {
    fn set_link(&mut self, link: Option<String>) {
        if self.link == link {
            return;
        }
        if let Some(old) = self.link.take() {
            self.out.push_str("](");
            self.out.push_str(&link_dest(&old));
            self.out.push(')');
        }
        if link.is_some() {
            self.out.push('[');
        }
        self.link = link;
    }

    fn set_status(&mut self, st: Status) {
        if self.st == st {
            return;
        }
        // Links never span a change boundary; the caller reopens them.
        self.set_link(None);
        match self.st {
            Status::Ins => self.out.push_str("++}"),
            Status::Del => self.out.push_str("--}"),
            Status::Plain => {}
        }
        match st {
            Status::Ins => self.out.push_str("{++"),
            Status::Del => self.out.push_str("{--"),
            Status::Plain => {}
        }
        self.st = st;
    }

    fn segment(&mut self, st: Status, link: Option<String>, marks: Marks, seg: &[(&str, &str)]) {
        if self.st != st {
            self.set_status(st);
        }
        self.set_link(link);
        let is_ws = |k: &str| k.chars().all(char::is_whitespace);
        let lead = seg.iter().take_while(|(k, _)| is_ws(k)).count();
        let trail = seg[lead..].iter().rev().take_while(|(k, _)| is_ws(k)).count();
        let core = &seg[lead..seg.len() - trail];
        self.atoms(&seg[..lead], false);
        if core.is_empty() {
            self.atoms(&seg[seg.len() - trail..], false);
            return;
        }
        let code = marks.code && !self.mode.code;
        let mut open = String::new();
        if marks.bold && !self.mode.code {
            open.push_str("**");
        }
        if marks.italic && !self.mode.code {
            open.push('*');
        }
        if marks.strike && !self.mode.code {
            open.push_str("~~");
        }
        self.out.push_str(&open);
        if code {
            // Objects can't live inside code spans; split around them.
            let mut buf = String::new();
            for (k, alt) in core {
                if k.chars().count() == 1 && *k != "\n" {
                    buf.push_str(k);
                } else {
                    self.code_span(&std::mem::take(&mut buf));
                    self.atom(k, alt);
                }
            }
            self.code_span(&buf);
        } else {
            self.atoms(core, true);
        }
        self.out.push_str(&open.chars().rev().collect::<String>());
        self.atoms(&seg[seg.len() - trail..], false);
    }

    fn code_span(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let mut longest = 0;
        let mut cur = 0;
        for c in s.chars() {
            if c == '`' {
                cur += 1;
                longest = longest.max(cur);
            } else {
                cur = 0;
            }
        }
        let fence = "`".repeat(longest + 1);
        let pad = s.starts_with('`') || s.ends_with('`') || (s.starts_with(' ') && s.ends_with(' '));
        let body = if self.mode.table { s.replace('|', "\\|") } else { s.to_string() };
        self.out.push_str(&fence);
        if pad {
            self.out.push(' ');
        }
        self.out.push_str(&body);
        if pad {
            self.out.push(' ');
        }
        self.out.push_str(&fence);
    }

    fn atoms(&mut self, seg: &[(&str, &str)], _core: bool) {
        let chars: Vec<char> = seg
            .iter()
            .map(|(k, _)| if k.chars().count() == 1 { k.chars().next().unwrap() } else { '\u{FFFC}' })
            .collect();
        let mut force = vec![false; seg.len()];
        for (i, (k, alt)) in seg.iter().enumerate() {
            if k.chars().count() == 1 && *k != "\n" {
                let c = chars[i];
                let prev = if i > 0 { Some(chars[i - 1]) } else { self.out.chars().last() };
                let next = chars.get(i + 1).copied();
                let line_start = self.out.is_empty() || self.out.ends_with('\n');
                // "1. Intro" at a line start must not turn into a list.
                if line_start && c.is_ascii_digit() && !self.mode.code {
                    let mut j = i;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    if j < chars.len() && (chars[j] == '.' || chars[j] == ')') {
                        force[j] = true;
                    }
                }
                if force[i] {
                    self.out.push('\\');
                    self.out.push(c);
                } else {
                    self.out.push_str(&escape_char(c, prev, next, line_start, self.mode));
                }
            } else {
                self.atom(k, alt);
            }
        }
    }

    fn atom(&mut self, k: &str, alt: &str) {
        if k == "\n" {
            if self.mode.table || self.mode.heading || self.mode.code {
                self.out.push_str("<br/>");
            } else {
                self.out.push_str("\\\n");
            }
        } else if k == "\t" {
            self.out.push('\t');
        } else if let Some(t) = k.strip_prefix("img:") {
            self.out.push_str(&format!("![{}]({})", escape_plain(alt), link_dest(t)));
        } else if let Some(n) = k.strip_prefix("fn:") {
            self.out.push_str(&format!("[^{n}]"));
        } else {
            self.out.push_str(k);
        }
    }
}

fn link_dest(url: &str) -> String {
    if url.contains([' ', '(', ')', '<', '>']) {
        format!("<{}>", url.replace('>', "%3E").replace('<', "%3C"))
    } else {
        url.to_string()
    }
}

fn escape_char(c: char, prev: Option<char>, next: Option<char>, line_start: bool, mode: Mode) -> String {
    if mode.code {
        return c.to_string();
    }
    let esc = match c {
        '\\' | '`' | '*' | '[' | ']' | '<' | '~' => true,
        '_' => !prev.is_some_and(char::is_alphanumeric) || !next.is_some_and(char::is_alphanumeric),
        '&' => next.is_some_and(|n| n.is_alphanumeric() || n == '#'),
        '{' => next.is_some_and(|n| "+-~=>".contains(n)),
        '}' => prev.is_some_and(|p| "+-~=<".contains(p)),
        '|' => mode.table,
        '#' => line_start || mode.heading,
        '>' | '-' | '+' | '=' => line_start,
        '!' => next == Some('['),
        _ => false,
    };
    if esc { format!("\\{c}") } else { c.to_string() }
}

pub fn escape_plain(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        out.push_str(&escape_char(*c, prev, chars.get(i + 1).copied(), i == 0, Mode::default()));
    }
    out
}
