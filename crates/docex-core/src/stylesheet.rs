//! Stylesheets: a document's look as a small subset of CSS, kept in Word's
//! own built-in styles (Normal, Heading 1–6, Title, Quote, Caption, …) and
//! the page setup, so Word, its navigation pane and table of contents keep
//! working, and Word in any language knows the styles ("Überschrift 1").
//!
//! [`parse`] reads the CSS; `Docx::apply_stylesheet` writes it into the
//! document; `Docx::stylesheet` reads a document's look back. [`Stylesheet::to_css`]
//! writes it as CSS again (to start a theme from a document), and
//! [`Stylesheet::editor_css`] for the editor, which shows the page that way.

use crate::xml::{Document, Element, Node};
use std::collections::BTreeMap;
use std::fmt::Write;

/// The themes that come with docex: name, description, CSS.
pub const BUILTIN: &[(&str, &str, &str)] = &[
    ("report", "Calibri, dark blue headings with a rule under the first level; for reports and memos", include_str!("../themes/report.css")),
    ("classic", "Georgia, justified, quiet headings in small capitals; for longer reading", include_str!("../themes/classic.css")),
    ("thesis", "Cambria 12 pt, 1.5 line spacing, numbered headings, wide margins; for theses and papers", include_str!("../themes/thesis.css")),
    ("plain", "Arial, black, no colour; neutral and safe for any recipient", include_str!("../themes/plain.css")),
];

/// The CSS of a built-in theme.
pub fn builtin(name: &str) -> Option<&'static str> {
    BUILTIN.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(name)).map(|(_, _, css)| *css)
}

/// What a rule styles: a Word built-in style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    Body,
    Title,
    Subtitle,
    Heading(u8),
    Quote,
    Caption,
    Code,
    Link,
}

impl Target {
    fn from_selector(s: &str) -> Option<Target> {
        let s = s.trim().to_ascii_lowercase();
        Some(match s.as_str() {
            "body" | "p" => Target::Body,
            ".title" => Target::Title,
            ".subtitle" => Target::Subtitle,
            "blockquote" | ".quote" => Target::Quote,
            "figcaption" | "caption" | ".caption" => Target::Caption,
            "pre" | "code" => Target::Code,
            "a" => Target::Link,
            _ => match s.strip_prefix('h').and_then(|n| n.parse::<u8>().ok()) {
                Some(n @ 1..=6) => Target::Heading(n),
                _ => return None,
            },
        })
    }

    fn selector(self) -> String {
        match self {
            Target::Body => "body".into(),
            Target::Title => ".title".into(),
            Target::Subtitle => ".subtitle".into(),
            Target::Heading(n) => format!("h{n}"),
            Target::Quote => "blockquote".into(),
            Target::Caption => "figcaption".into(),
            Target::Code => "pre".into(),
            Target::Link => "a".into(),
        }
    }

    /// Word style names that count as this target (lowercase), the first one
    /// used for a new style, and the id a new style gets.
    fn word(self) -> (Vec<String>, String) {
        let (names, id): (&[&str], &str) = match self {
            Target::Body => (&["normal"], "Normal"),
            Target::Title => (&["title"], "Title"),
            Target::Subtitle => (&["subtitle"], "Subtitle"),
            Target::Heading(n) => return (vec![format!("heading {n}")], format!("Heading{n}")),
            Target::Quote => (&["quote", "block text"], "Quote"),
            Target::Caption => (&["caption"], "Caption"),
            Target::Code => (&["source code", "code", "html preformatted", "plain text"], "SourceCode"),
            Target::Link => (&["hyperlink"], "Hyperlink"),
        };
        (names.iter().map(|s| s.to_string()).collect(), id.to_string())
    }

    /// Properties a style takes from the body, as CSS inherits them.
    fn inherits(self) -> bool {
        !matches!(self, Target::Link)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Line {
    /// A multiple of single spacing (1.15).
    Multiple(f32),
    /// At least this many points.
    AtLeast(f32),
}

/// What a rule sets. `None`: not set (Word's style keeps what it has).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    /// The font families as CSS lists them; Word uses the first.
    pub family: Option<String>,
    /// Points.
    pub size: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub caps: Option<bool>,
    pub small_caps: Option<bool>,
    pub underline: Option<bool>,
    /// `RRGGBB`.
    pub color: Option<String>,
    pub background: Option<String>,
    /// left, center, right, justify.
    pub align: Option<String>,
    pub line: Option<Line>,
    /// Space before and after, left indent, first-line indent, letter spacing: points.
    pub before: Option<f32>,
    pub after: Option<f32>,
    pub indent: Option<f32>,
    pub first_line: Option<f32>,
    pub letter: Option<f32>,
    /// Width in points and colour of a rule under the paragraph.
    pub rule: Option<(f32, String)>,
    pub page_break: Option<bool>,
    /// Headings: numbered (1, 1.1, 1.1.1).
    pub numbered: Option<bool>,
}

impl Look {
    /// The first family, as Word names it.
    pub fn font(&self) -> Option<String> {
        let f = self.family.as_deref()?.split(',').next()?.trim().trim_matches(['"', '\'']).trim();
        (!f.is_empty() && !is_generic(f)).then(|| f.to_string())
    }

    /// This look with what it doesn't set taken from `body`.
    fn inheriting(&self, body: &Look) -> Look {
        let mut l = self.clone();
        l.family = l.family.or(body.family.clone());
        l.size = l.size.or(body.size);
        l.color = l.color.or(body.color.clone());
        l.line = l.line.or(body.line);
        l.align = l.align.or(body.align.clone());
        l
    }
}

fn is_generic(f: &str) -> bool {
    matches!(f, "serif" | "sans-serif" | "monospace" | "system-ui" | "cursive" | "fantasy" | "ui-serif" | "ui-sans-serif" | "ui-monospace")
}

/// Page size and margins in points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub size: Option<(f32, f32)>,
    /// Top, right, bottom, left.
    pub margin: Option<[f32; 4]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stylesheet {
    pub page: Page,
    pub rules: BTreeMap<Target, Look>,
}

// ---------------------------------------------------------------------------
// CSS in

const PT_PER_CM: f32 = 72.0 / 2.54;

/// A length in points; `em` is relative to `em` points.
fn length(v: &str, em: f32) -> Option<f32> {
    let v = v.trim();
    if v == "0" {
        return Some(0.0);
    }
    let split = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(v.len());
    let n: f32 = v[..split].parse().ok()?;
    Some(match &v[split..] {
        "pt" => n,
        "px" => n * 0.75,
        "cm" => n * PT_PER_CM,
        "mm" => n * PT_PER_CM / 10.0,
        "in" => n * 72.0,
        "em" | "rem" => n * em,
        _ => return None,
    })
}

fn color(v: &str) -> Option<String> {
    let v = v.trim().to_ascii_lowercase();
    let hex = match v.as_str() {
        "black" => "000000",
        "white" => "FFFFFF",
        "gray" | "grey" => "808080",
        "red" => "FF0000",
        "blue" => "0000FF",
        "navy" => "000080",
        "green" => "008000",
        _ => {
            let h = v.strip_prefix('#')?;
            return match h.len() {
                3 => Some(h.chars().flat_map(|c| [c, c]).collect::<String>().to_ascii_uppercase()),
                6 if h.chars().all(|c| c.is_ascii_hexdigit()) => Some(h.to_ascii_uppercase()),
                _ => None,
            };
        }
    };
    Some(hex.into())
}

/// Top, right, bottom, left of a 1–4 value shorthand.
fn four(v: &str, em: f32) -> Option<[f32; 4]> {
    let p: Vec<f32> = v.split_whitespace().map(|x| length(x, em)).collect::<Option<_>>()?;
    Some(match p.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return None,
    })
}

fn page_size(v: &str) -> Option<(f32, f32)> {
    let v = v.trim().to_ascii_lowercase();
    let landscape = v.contains("landscape");
    let named = |s: &str| -> Option<(f32, f32)> {
        Some(match s {
            "a4" => (21.0 * PT_PER_CM, 29.7 * PT_PER_CM),
            "a5" => (14.8 * PT_PER_CM, 21.0 * PT_PER_CM),
            "a3" => (29.7 * PT_PER_CM, 42.0 * PT_PER_CM),
            "letter" => (612.0, 792.0),
            "legal" => (612.0, 1008.0),
            _ => return None,
        })
    };
    let words: Vec<&str> = v.split_whitespace().filter(|w| *w != "landscape" && *w != "portrait").collect();
    let (w, h) = match words.as_slice() {
        [n] => named(n)?,
        [a, b] => (length(a, 12.0)?, length(b, 12.0)?),
        _ => return None,
    };
    Some(if landscape { (w.max(h), w.min(h)) } else { (w, h) })
}

/// Split declarations at `;`, outside quotes.
fn declarations(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = None;
    for c in body.chars().chain([';']) {
        match c {
            '"' | '\'' if quote.is_none() => quote = Some(c),
            c2 if Some(c2) == quote => quote = None,
            ';' if quote.is_none() => {
                if let Some((k, v)) = cur.split_once(':') {
                    out.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
                }
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out
}

/// The CSS rules: `(selectors, declarations)`, comments removed.
fn rules(css: &str) -> Vec<(Vec<String>, Vec<(String, String)>)> {
    let mut s = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        s.push_str(&rest[..i]);
        rest = rest[i + 2..].split_once("*/").map_or("", |(_, r)| r);
    }
    s.push_str(rest);
    s.split('}')
        .filter_map(|r| r.split_once('{'))
        .map(|(sel, body)| (sel.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(), declarations(body)))
        .collect()
}

/// Read a stylesheet. Unknown selectors and properties are reported, not fatal.
pub fn parse(css: &str) -> (Stylesheet, Vec<String>) {
    let mut sheet = Stylesheet::default();
    let mut warnings = Vec::new();
    let mut decls: BTreeMap<Target, Vec<(String, String)>> = BTreeMap::new();
    for (sels, ds) in rules(css) {
        if sels.iter().any(|s| s.eq_ignore_ascii_case("@page")) {
            for (k, v) in &ds {
                match k.as_str() {
                    "size" => sheet.page.size = page_size(v).or_else(|| bad(&mut warnings, "@page", k, v)),
                    "margin" => sheet.page.margin = four(v, 12.0).or_else(|| bad(&mut warnings, "@page", k, v)),
                    _ => warnings.push(format!("@page {{ {k} }} is not supported; ignored")),
                }
            }
            continue;
        }
        for s in &sels {
            match Target::from_selector(s) {
                Some(t) => decls.entry(t).or_default().extend(ds.iter().cloned()),
                None => warnings.push(format!("selector \"{s}\" is not supported; ignored")),
            }
        }
    }
    // Font sizes first: `em` lengths depend on them.
    let size_of = |ds: &[(String, String)], base: f32| {
        let mut size = None;
        for (k, v) in ds {
            if k == "font-size" {
                size = length(v, base).or(size);
            } else if k == "font" {
                size = font_shorthand(v, base).1.or(size);
            }
        }
        size
    };
    let body_size = decls.get(&Target::Body).and_then(|d| size_of(d, 11.0)).unwrap_or(11.0);
    for (t, ds) in &decls {
        let em = size_of(ds, body_size).unwrap_or(body_size);
        let mut l = Look::default();
        for (k, v) in ds {
            if !set(&mut l, k, v, em, body_size) {
                warnings.push(format!("{} {{ {k}: {v} }} is not supported; ignored", t.selector()));
            }
        }
        sheet.rules.insert(*t, l);
    }
    (sheet, warnings)
}

fn bad<T>(w: &mut Vec<String>, sel: &str, k: &str, v: &str) -> Option<T> {
    w.push(format!("{sel} {{ {k}: {v} }} can't be read; ignored"));
    None
}

/// `font: [italic] [bold] SIZE[/LINE] FAMILY`: (italic, size, line, bold, family).
fn font_shorthand(v: &str, base: f32) -> (Option<bool>, Option<f32>, Option<Line>, Option<bool>, Option<String>) {
    let (mut italic, mut size, mut line, mut bold) = (None, None, None, None);
    let mut rest = v.trim();
    while let Some((word, tail)) = rest.split_once(char::is_whitespace) {
        match word {
            "italic" | "oblique" => italic = Some(true),
            "bold" | "600" | "700" | "800" | "900" => bold = Some(true),
            "normal" | "400" => {}
            w => {
                let (s, l) = w.split_once('/').map_or((w, None), |(s, l)| (s, Some(l)));
                match length(s, base) {
                    Some(px) => {
                        size = Some(px);
                        line = l.and_then(|l| line_height(l, px));
                        rest = tail.trim();
                    }
                    None => {}
                }
                break;
            }
        }
        rest = tail.trim();
    }
    let family = (!rest.is_empty()).then(|| rest.to_string());
    (italic, size, line, bold, family)
}

fn line_height(v: &str, em: f32) -> Option<Line> {
    let v = v.trim();
    if v == "normal" {
        return Some(Line::Multiple(1.0));
    }
    if let Some(p) = v.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().map(|p| Line::Multiple(p / 100.0));
    }
    if let Ok(n) = v.parse::<f32>() {
        return Some(Line::Multiple(n));
    }
    length(v, em).map(Line::AtLeast)
}

/// Apply one declaration; false if it isn't supported.
fn set(l: &mut Look, k: &str, v: &str, em: f32, body: f32) -> bool {
    let on = |v: &str, yes: &[&str]| yes.contains(&v.trim());
    match k {
        "font-family" => l.family = Some(v.to_string()),
        "font-size" => l.size = length(v, body),
        "font-weight" => l.bold = Some(on(v, &["bold", "bolder", "600", "700", "800", "900"])),
        "font-style" => l.italic = Some(on(v, &["italic", "oblique"])),
        "font-variant" => l.small_caps = Some(on(v, &["small-caps"])),
        "font" => {
            let (i, s, ln, b, f) = font_shorthand(v, body);
            l.italic = i.or(l.italic);
            l.size = s.or(l.size);
            l.line = ln.or(l.line);
            l.bold = b.or(l.bold);
            l.family = f.or(l.family.take());
        }
        "text-transform" => l.caps = Some(on(v, &["uppercase"])),
        "text-decoration" | "text-decoration-line" => l.underline = Some(v.contains("underline")),
        "color" => return color(v).map(|c| l.color = Some(c)).is_some(),
        "background" | "background-color" => return color(v).map(|c| l.background = Some(c)).is_some(),
        "text-align" => match v.trim() {
            a @ ("left" | "center" | "right" | "justify") => l.align = Some(a.into()),
            "start" => l.align = Some("left".into()),
            "end" => l.align = Some("right".into()),
            _ => return false,
        },
        "line-height" => return line_height(v, em).map(|x| l.line = Some(x)).is_some(),
        "margin-top" => return length(v, em).map(|x| l.before = Some(x)).is_some(),
        "margin-bottom" => return length(v, em).map(|x| l.after = Some(x)).is_some(),
        "margin-left" | "padding-left" => return length(v, em).map(|x| l.indent = Some(x)).is_some(),
        "margin" => match four(v, em) {
            Some([t, _, b, left]) => {
                l.before = Some(t);
                l.after = Some(b);
                l.indent = Some(left);
            }
            None => return false,
        },
        "text-indent" => return length(v, em).map(|x| l.first_line = Some(x)).is_some(),
        "letter-spacing" => return length(v, em).map(|x| l.letter = Some(x)).is_some(),
        "border-bottom" => {
            if v.trim() == "none" {
                l.rule = Some((0.0, String::new()));
                return true;
            }
            let w = v.split_whitespace().find_map(|x| length(x, em)).unwrap_or(0.75);
            let c = v.split_whitespace().find_map(color).unwrap_or_else(|| "000000".into());
            l.rule = Some((w, c));
        }
        "page-break-before" | "break-before" => l.page_break = Some(on(v, &["always", "page"])),
        "counter-increment" => l.numbered = Some(v.trim() != "none"),
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------------------
// CSS out

fn num(x: f32) -> String {
    let s = format!("{:.2}", x);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn pt(x: f32) -> String {
    if x == 0.0 {
        "0".into()
    } else {
        format!("{}pt", num(x))
    }
}

fn cm(x: f32) -> String {
    format!("{}cm", num(x / PT_PER_CM))
}

/// A Word font as a CSS family list: with a look-alike that may be installed
/// where Word's isn't (Carlito for Calibri, …) and a generic family.
pub fn css_family(font: &str) -> String {
    let alike: &[&str] = match font {
        "Calibri" | "Calibri Light" => &["Carlito"],
        "Cambria" => &["Caladea"],
        "Arial" => &["Liberation Sans", "Arimo"],
        "Times New Roman" => &["Liberation Serif", "Tinos"],
        "Courier New" => &["Liberation Mono", "Cousine"],
        "Georgia" => &["Gelasio"],
        "Aptos" | "Aptos Display" => &["Inter"],
        _ => &[],
    };
    let lower = font.to_ascii_lowercase();
    let generic = if ["mono", "courier", "consolas", "code"].iter().any(|m| lower.contains(m)) {
        "monospace"
    } else if !lower.contains("sans") && ["times", "georgia", "cambria", "garamond", "serif", "book", "palatino", "minion", "caslon", "baskerville", "caladea", "gelasio"].iter().any(|m| lower.contains(m)) {
        "serif"
    } else {
        "sans-serif"
    };
    let q = |f: &str| if f.contains(' ') { format!("\"{f}\"") } else { f.to_string() };
    std::iter::once(q(font)).chain(alike.iter().map(|f| q(f))).chain([generic.to_string()]).collect::<Vec<_>>().join(", ")
}

impl Stylesheet {
    /// Each rule with what it inherits from the body.
    fn resolved(&self, t: Target) -> Option<Look> {
        let l = self.rules.get(&t)?;
        let body = self.rules.get(&Target::Body).cloned().unwrap_or_default();
        let mut l = if t.inherits() && t != Target::Body { l.inheriting(&body) } else { l.clone() };
        // As in CSS: weight, style and capitals come from the body, else normal.
        if t.inherits() {
            l.bold = l.bold.or(body.bold).or(Some(false));
            l.italic = l.italic.or(body.italic).or(Some(false));
            l.caps = l.caps.or(body.caps).or(Some(false));
            l.small_caps = l.small_caps.or(body.small_caps).or(Some(false));
        }
        Some(l)
    }

    /// As a stylesheet to read and edit.
    pub fn to_css(&self) -> String {
        let mut s = String::new();
        if self.page.size.is_some() || self.page.margin.is_some() {
            s.push_str("@page {\n");
            if let Some((w, h)) = self.page.size {
                let a4 = (21.0 * PT_PER_CM, 29.7 * PT_PER_CM);
                if (w - a4.0).abs() < 2.0 && (h - a4.1).abs() < 2.0 {
                    s.push_str("  size: A4;\n");
                } else if (w - 612.0).abs() < 2.0 && (h - 792.0).abs() < 2.0 {
                    s.push_str("  size: letter;\n");
                } else {
                    let _ = writeln!(s, "  size: {} {};", cm(w), cm(h));
                }
            }
            if let Some([t, r, b, l]) = self.page.margin {
                let _ = writeln!(s, "  margin: {} {} {} {};", cm(t), cm(r), cm(b), cm(l));
            }
            s.push_str("}\n\n");
        }
        for (t, l) in &self.rules {
            let d = declarations_of(l);
            if d.is_empty() {
                continue;
            }
            let _ = writeln!(s, "{} {{", t.selector());
            for (k, v) in d {
                let _ = writeln!(s, "  {k}: {v};");
            }
            s.push_str("}\n\n");
        }
        s.trim_end().to_string() + "\n"
    }

    /// For the editor: the page drawn this way. Colours apply only with
    /// `.print` on the page (otherwise the app's theme colours stay).
    pub fn editor_css(&self) -> String {
        let mut s = String::new();
        let pg = ".page.look";
        let pm = format!("{pg} .ProseMirror");
        if let Some((w, _)) = self.page.size {
            let _ = writeln!(s, "{pg} {{ max-width: {}px; }}", num(w / 0.75));
        }
        if let Some([t, r, b, l]) = self.page.margin {
            // The margins shrink with the page in a narrow window, as a share
            // of its width (a percentage padding is one of the page-wrap's
            // width, which is the page's once the page no longer fits).
            let w = self.page.size.map_or(615.0, |(w, _)| w);
            let px = |x: f32| format!("min({}px, {}%)", num(x / 0.75), num(x / w * 100.0));
            let _ = writeln!(s, "{pg} {{ padding: {} {} {} {}; }}", px(t), px(r), px(b), px(l));
        }
        for t in self.rules.keys() {
            let l = self.resolved(*t).unwrap_or_default();
            let (sel, inner): (String, Option<String>) = match t {
                Target::Body => (pm.clone(), Some(format!("{pm} p.k-normal"))),
                Target::Heading(n) => (format!("{pm} h{n}"), None),
                Target::Quote => (format!("{pm} p.k-quote"), None),
                Target::Caption => (format!("{pm} p.k-caption"), None),
                Target::Code => (format!("{pm} p.k-code"), None),
                Target::Link => (format!("{pm} a"), None),
                // The editor has no title paragraphs (they show as headings).
                Target::Title | Target::Subtitle => continue,
            };
            let mut text = Vec::new();
            let mut para = Vec::new();
            if let Some(f) = &l.family {
                let fam = match (l.font(), f.contains(',')) {
                    (Some(word), false) => css_family(&word),
                    _ => f.clone(),
                };
                text.push(format!("font-family: {fam}"));
            }
            if let Some(x) = l.size {
                text.push(format!("font-size: {}", pt(x)));
            }
            if let Some(b) = l.bold {
                text.push(format!("font-weight: {}", if b { 700 } else { 400 }));
            }
            if let Some(i) = l.italic {
                text.push(format!("font-style: {}", if i { "italic" } else { "normal" }));
            }
            if let Some(c) = l.caps {
                text.push(format!("text-transform: {}", if c { "uppercase" } else { "none" }));
            }
            if let Some(c) = l.small_caps {
                text.push(format!("font-variant: {}", if c { "small-caps" } else { "normal" }));
            }
            if let Some(x) = l.letter {
                text.push(format!("letter-spacing: {}", pt(x)));
            }
            match l.line {
                Some(Line::Multiple(m)) => text.push(format!("line-height: {}", num(m * 1.17))),
                Some(Line::AtLeast(x)) => text.push(format!("line-height: {}", pt(x))),
                None => {}
            }
            if let Some(a) = &l.align {
                para.push(format!("text-align: {a}"));
            }
            if l.before.is_some() || l.after.is_some() {
                para.push(format!("margin-top: {}; margin-bottom: {}", pt(l.before.unwrap_or(0.0)), pt(l.after.unwrap_or(0.0))));
            }
            if let Some(x) = l.indent {
                para.push(format!("margin-left: {}; padding-left: 0; border-left: 0", pt(x)));
            }
            if let Some(x) = l.first_line {
                para.push(format!("text-indent: {}", pt(x)));
            }
            if let Some((w, _)) = &l.rule {
                para.push(if *w > 0.0 { format!("border-bottom: {} solid color-mix(in srgb, currentColor 35%, transparent); padding-bottom: 2pt", pt(*w)) } else { "border-bottom: 0".into() });
            }
            if *t == Target::Link {
                if let Some(u) = l.underline {
                    text.push(format!("text-decoration: {}", if u { "underline" } else { "none" }));
                }
            }
            match &inner {
                Some(i) => {
                    rule(&mut s, &sel, &text);
                    rule(&mut s, i, &para);
                }
                None => rule(&mut s, &sel, &[text, para].concat()),
            }
            // Colours: only for the page in print colours.
            let print = sel.replacen(pg, &format!("{pg}.print"), 1);
            let mut colours = Vec::new();
            if let Some(c) = &l.color {
                colours.push(format!("color: #{c}"));
            }
            if let Some(c) = &l.background {
                colours.push(format!("background: #{c}"));
            }
            if let Some((w, c)) = &l.rule {
                if *w > 0.0 {
                    colours.push(format!("border-bottom-color: #{c}"));
                }
            }
            rule(&mut s, &print, &colours);
        }
        s
    }
}

fn rule(s: &mut String, sel: &str, decls: &[String]) {
    if !decls.is_empty() {
        let _ = writeln!(s, "{sel} {{ {}; }}", decls.join("; "));
    }
}

fn declarations_of(l: &Look) -> Vec<(&'static str, String)> {
    let mut d = Vec::new();
    if let Some(f) = &l.family {
        d.push(("font-family", f.clone()));
    }
    if let Some(x) = l.size {
        d.push(("font-size", pt(x)));
    }
    if let Some(b) = l.bold {
        d.push(("font-weight", if b { "bold" } else { "normal" }.into()));
    }
    if let Some(i) = l.italic {
        d.push(("font-style", if i { "italic" } else { "normal" }.into()));
    }
    if let Some(c) = l.small_caps {
        d.push(("font-variant", if c { "small-caps" } else { "normal" }.into()));
    }
    if let Some(c) = l.caps {
        d.push(("text-transform", if c { "uppercase" } else { "none" }.into()));
    }
    if let Some(u) = l.underline {
        d.push(("text-decoration", if u { "underline" } else { "none" }.into()));
    }
    if let Some(x) = l.letter {
        d.push(("letter-spacing", pt(x)));
    }
    if let Some(c) = &l.color {
        d.push(("color", format!("#{c}")));
    }
    if let Some(c) = &l.background {
        d.push(("background", format!("#{c}")));
    }
    match l.line {
        Some(Line::Multiple(m)) => d.push(("line-height", num(m))),
        Some(Line::AtLeast(x)) => d.push(("line-height", pt(x))),
        None => {}
    }
    if let Some(a) = &l.align {
        d.push(("text-align", a.clone()));
    }
    if let Some(x) = l.before {
        d.push(("margin-top", pt(x)));
    }
    if let Some(x) = l.after {
        d.push(("margin-bottom", pt(x)));
    }
    if let Some(x) = l.indent {
        d.push(("margin-left", pt(x)));
    }
    if let Some(x) = l.first_line {
        d.push(("text-indent", pt(x)));
    }
    if let Some((w, c)) = &l.rule {
        d.push(("border-bottom", if *w > 0.0 { format!("{} solid #{c}", pt(*w)) } else { "none".into() }));
    }
    if let Some(p) = l.page_break {
        d.push(("break-before", if p { "page" } else { "auto" }.into()));
    }
    if let Some(n) = l.numbered {
        d.push(("counter-increment", if n { "section" } else { "none" }.into()));
    }
    d
}

// ---------------------------------------------------------------------------
// Word in and out

const STYLE_ORDER: &[&str] = &[
    "w:name", "w:aliases", "w:basedOn", "w:next", "w:link", "w:autoRedefine", "w:hidden", "w:uiPriority",
    "w:semiHidden", "w:unhideWhenUsed", "w:qFormat", "w:locked", "w:personal", "w:personalCompose",
    "w:personalReply", "w:rsid", "w:pPr", "w:rPr", "w:tblPr", "w:trPr", "w:tcPr", "w:tblStylePr",
];

const SECT_ORDER: &[&str] = &[
    "w:headerReference", "w:footerReference", "w:footnotePr", "w:endnotePr", "w:type", "w:pgSz", "w:pgMar",
    "w:paperSrc", "w:pgBorders", "w:lnNumType", "w:pgNumType", "w:cols", "w:formProt", "w:vAlign",
    "w:noEndnote", "w:titlePg", "w:textDirection", "w:bidi", "w:rtlGutter", "w:docGrid", "w:printerSettings",
    "w:sectPrChange",
];

use crate::patch::{PPR_ORDER, RPR_ORDER};

fn twips(x: f32) -> String {
    ((x * 20.0).round() as i64).to_string()
}

fn style_name(e: &Element) -> String {
    e.child("w:name").and_then(|n| n.attr("w:val")).unwrap_or("").to_ascii_lowercase()
}

/// Index of the style for `t` among the styles root's children.
fn find_style(root: &Element, t: Target) -> Option<usize> {
    let (names, _) = t.word();
    let char_style = t == Target::Link;
    let is = |e: &Element| e.is("w:style") && (e.attr("w:type") == Some("character")) == char_style;
    names.iter().find_map(|n| root.children.iter().position(|c| matches!(c, Node::Element(e) if is(e) && style_name(e) == *n)))
}

fn new_style(root: &Element, t: Target) -> Element {
    let (names, mut id) = t.word();
    while root.elements().any(|e| e.is("w:style") && e.attr("w:styleId") == Some(id.as_str())) {
        id.push('1');
    }
    // Word's names for its built-in styles.
    let name = match t {
        Target::Body => "Normal".into(),
        Target::Title => "Title".into(),
        Target::Subtitle => "Subtitle".into(),
        Target::Quote => "Quote".into(),
        Target::Code => "Source Code".into(),
        Target::Link => "Hyperlink".into(),
        _ => names[0].clone(),
    };
    let mut e = Element::new("w:style").with_attr("w:type", if t == Target::Link { "character" } else { "paragraph" });
    if t == Target::Body {
        e.set_attr("w:default", "1");
    }
    e.set_attr("w:styleId", id);
    e.children.push(Element::new("w:name").with_attr("w:val", name).into());
    if !matches!(t, Target::Body | Target::Link) {
        e.children.push(Element::new("w:basedOn").with_attr("w:val", "Normal").into());
        e.children.push(Element::new("w:next").with_attr("w:val", "Normal").into());
    }
    if t != Target::Code && t != Target::Link {
        e.children.push(Element::new("w:qFormat").into());
    }
    if let Target::Heading(n) = t {
        e.children.push(
            Element::new("w:pPr")
                .with_child(Element::new("w:keepNext"))
                .with_child(Element::new("w:keepLines"))
                .with_child(Element::new("w:outlineLvl").with_attr("w:val", (n - 1).to_string()))
                .into(),
        );
    }
    e
}

fn child_or_new<'a>(e: &'a mut Element, name: &str, order: &[&str]) -> &'a mut Element {
    if e.child(name).is_none() {
        e.insert_ordered(Element::new(name), order);
    }
    e.child_mut(name).unwrap()
}

fn toggle(rpr: &mut Element, name: &str, on: bool) {
    let el = if on { Element::new(name) } else { Element::new(name).with_attr("w:val", "0") };
    rpr.insert_ordered(el, RPR_ORDER);
}

fn write_rpr(rpr: &mut Element, l: &Look) {
    if let Some(f) = l.font() {
        rpr.insert_ordered(
            Element::new("w:rFonts").with_attr("w:ascii", f.clone()).with_attr("w:hAnsi", f.clone()).with_attr("w:eastAsia", f.clone()).with_attr("w:cs", f),
            RPR_ORDER,
        );
    }
    if let Some(b) = l.bold {
        toggle(rpr, "w:b", b);
        toggle(rpr, "w:bCs", b);
    }
    if let Some(i) = l.italic {
        toggle(rpr, "w:i", i);
        toggle(rpr, "w:iCs", i);
    }
    if let Some(c) = l.caps {
        toggle(rpr, "w:caps", c);
    }
    if let Some(c) = l.small_caps {
        toggle(rpr, "w:smallCaps", c);
    }
    if let Some(c) = &l.color {
        rpr.insert_ordered(Element::new("w:color").with_attr("w:val", c.clone()), RPR_ORDER);
    }
    if let Some(x) = l.letter {
        rpr.insert_ordered(Element::new("w:spacing").with_attr("w:val", twips(x)), RPR_ORDER);
    }
    if let Some(x) = l.size {
        let half = ((x * 2.0).round() as i64).to_string();
        rpr.insert_ordered(Element::new("w:sz").with_attr("w:val", half.clone()), RPR_ORDER);
        rpr.insert_ordered(Element::new("w:szCs").with_attr("w:val", half), RPR_ORDER);
    }
    if let Some(u) = l.underline {
        rpr.insert_ordered(Element::new("w:u").with_attr("w:val", if u { "single" } else { "none" }), RPR_ORDER);
    }
}

fn write_ppr(ppr: &mut Element, l: &Look) {
    if let Some(p) = l.page_break {
        if p {
            ppr.insert_ordered(Element::new("w:pageBreakBefore"), PPR_ORDER);
        } else {
            ppr.remove_children("w:pageBreakBefore");
        }
    }
    if let Some((w, c)) = &l.rule {
        if *w > 0.0 {
            let bottom = Element::new("w:bottom")
                .with_attr("w:val", "single")
                .with_attr("w:sz", ((w * 8.0).round().max(2.0) as i64).to_string())
                .with_attr("w:space", "1")
                .with_attr("w:color", c.clone());
            ppr.insert_ordered(Element::new("w:pBdr").with_child(bottom), PPR_ORDER);
        } else {
            ppr.remove_children("w:pBdr");
        }
    }
    if let Some(c) = &l.background {
        ppr.insert_ordered(Element::new("w:shd").with_attr("w:val", "clear").with_attr("w:color", "auto").with_attr("w:fill", c.clone()), PPR_ORDER);
    }
    if l.before.is_some() || l.after.is_some() || l.line.is_some() {
        let sp = child_or_new(ppr, "w:spacing", PPR_ORDER);
        if let Some(x) = l.before {
            sp.set_attr("w:before", twips(x));
            sp.remove_attr("w:beforeAutospacing");
            sp.remove_attr("w:beforeLines");
        }
        if let Some(x) = l.after {
            sp.set_attr("w:after", twips(x));
            sp.remove_attr("w:afterAutospacing");
            sp.remove_attr("w:afterLines");
        }
        match l.line {
            Some(Line::Multiple(m)) => {
                sp.set_attr("w:line", ((m * 240.0).round() as i64).to_string());
                sp.set_attr("w:lineRule", "auto");
            }
            Some(Line::AtLeast(x)) => {
                sp.set_attr("w:line", twips(x));
                sp.set_attr("w:lineRule", "atLeast");
            }
            None => {}
        }
    }
    if l.indent.is_some() || l.first_line.is_some() {
        let ind = child_or_new(ppr, "w:ind", PPR_ORDER);
        if let Some(x) = l.indent {
            ind.set_attr("w:left", twips(x));
            ind.remove_attr("w:start");
        }
        if let Some(x) = l.first_line {
            ind.remove_attr("w:firstLine");
            ind.remove_attr("w:hanging");
            if x >= 0.0 {
                ind.set_attr("w:firstLine", twips(x));
            } else {
                ind.set_attr("w:hanging", twips(-x));
            }
        }
    }
    if let Some(a) = &l.align {
        let v = match a.as_str() {
            "justify" => "both",
            a => a,
        };
        ppr.insert_ordered(Element::new("w:jc").with_attr("w:val", v), PPR_ORDER);
    }
}

/// Fonts of the document's font theme: (major = headings, minor = body).
pub fn theme_fonts(theme: Option<&Document>) -> (Option<String>, Option<String>) {
    let face = |which: &str| {
        let mut out = None;
        if let Some(t) = theme {
            t.root.visit(&mut |e| {
                if out.is_none() && e.is(which) {
                    out = e.child("a:latin").and_then(|l| l.attr("typeface")).filter(|f| !f.is_empty()).map(String::from);
                }
            });
        }
        out
    };
    (face("a:majorFont"), face("a:minorFont"))
}

/// Make the styles root follow `sheet`. Heading numbering is returned for the
/// numbering part: the levels to number (0-based) and the levels to un-number.
pub(crate) fn write_styles(root: &mut Element, sheet: &Stylesheet) -> (Vec<u8>, Vec<u8>) {
    let (mut number, mut unnumber) = (Vec::new(), Vec::new());
    if let Some(body) = sheet.rules.get(&Target::Body) {
        // Defaults too, for styles that aren't based on Normal.
        let defaults = child_or_new(root, "w:docDefaults", &["w:docDefaults", "w:latentStyles", "w:style"]);
        let rpr_default = child_or_new(defaults, "w:rPrDefault", &["w:rPrDefault", "w:pPrDefault"]);
        let rpr = child_or_new(rpr_default, "w:rPr", &["w:rPr"]);
        write_rpr(rpr, &Look { family: body.family.clone(), size: body.size, ..Look::default() });
    }
    for t in sheet.rules.keys() {
        let Some(l) = sheet.resolved(*t) else { continue };
        let i = match find_style(root, *t) {
            Some(i) => i,
            None => {
                let e = new_style(root, *t);
                root.children.push(e.into());
                root.children.len() - 1
            }
        };
        let Some(Node::Element(st)) = root.children.get_mut(i) else { continue };
        if *t != Target::Link {
            write_ppr(child_or_new(st, "w:pPr", STYLE_ORDER), &l);
        }
        write_rpr(child_or_new(st, "w:rPr", STYLE_ORDER), &l);
        if let (Target::Heading(n), Some(on)) = (t, l.numbered) {
            if on {
                number.push(n - 1);
            } else {
                unnumber.push(n - 1);
                if let Some(p) = st.child_mut("w:pPr") {
                    p.remove_children("w:numPr");
                }
            }
        }
    }
    (number, unnumber)
}

/// Number the heading styles for `levels`: a new multi-level list (1, 1.1,
/// 1.1.1) linked to Heading 1–6, unless they are numbered already.
pub(crate) fn number_headings(styles: &mut Element, numbering: &mut Element, levels: &[u8]) -> bool {
    let heading = |root: &Element, n: u8| find_style(root, Target::Heading(n + 1));
    let numbered = levels.iter().any(|l| {
        heading(styles, *l).and_then(|i| styles.children[i].as_element()?.child_path(&["w:pPr", "w:numPr"]).map(|_| ())).is_some()
    });
    if levels.is_empty() || numbered {
        return false;
    }
    let next = |name: &str, attr: &str| numbering.elements().filter(|e| e.is(name)).filter_map(|e| e.attr(attr)?.parse::<u32>().ok()).max().map_or(1, |m| m + 1);
    let abs_id = next("w:abstractNum", "w:abstractNumId");
    let num_id = next("w:num", "w:numId");
    let mut abs = Element::new("w:abstractNum").with_attr("w:abstractNumId", abs_id.to_string());
    abs.children.push(Element::new("w:multiLevelType").with_attr("w:val", "multilevel").into());
    for l in 0..9u8 {
        let text: Vec<String> = (1..=l + 1).map(|k| format!("%{k}")).collect();
        let mut lvl = Element::new("w:lvl")
            .with_attr("w:ilvl", l.to_string())
            .with_child(Element::new("w:start").with_attr("w:val", "1"))
            .with_child(Element::new("w:numFmt").with_attr("w:val", "decimal"));
        if l < 6 {
            if let Some(id) = heading(styles, l).and_then(|i| styles.children[i].as_element()?.attr("w:styleId").map(String::from)) {
                lvl.children.push(Element::new("w:pStyle").with_attr("w:val", id).into());
            }
        }
        let hang = 432 + 144 * l as u32;
        lvl.children.push(Element::new("w:lvlText").with_attr("w:val", text.join(".")).into());
        lvl.children.push(Element::new("w:lvlJc").with_attr("w:val", "left").into());
        lvl.children.push(Element::new("w:pPr").with_child(Element::new("w:ind").with_attr("w:left", hang.to_string()).with_attr("w:hanging", hang.to_string())).into());
        abs.children.push(lvl.into());
    }
    let pos = numbering.children.iter().position(|c| matches!(c, Node::Element(e) if e.is("w:num"))).unwrap_or(numbering.children.len());
    numbering.children.insert(pos, abs.into());
    numbering.children.push(
        Element::new("w:num").with_attr("w:numId", num_id.to_string()).with_child(Element::new("w:abstractNumId").with_attr("w:val", abs_id.to_string())).into(),
    );
    for l in levels {
        if let Some(i) = heading(styles, *l) {
            if let Some(Node::Element(st)) = styles.children.get_mut(i) {
                let numpr = Element::new("w:numPr")
                    .with_child(Element::new("w:ilvl").with_attr("w:val", l.to_string()))
                    .with_child(Element::new("w:numId").with_attr("w:val", num_id.to_string()));
                child_or_new(st, "w:pPr", STYLE_ORDER).insert_ordered(numpr, PPR_ORDER);
            }
        }
    }
    true
}

/// Page size and margins in every section.
pub(crate) fn write_page(body: &mut Element, page: &Page) -> bool {
    if page.size.is_none() && page.margin.is_none() {
        return false;
    }
    body.visit_mut(&mut |e| {
        if !e.is("w:sectPr") {
            return;
        }
        if let Some((w, h)) = page.size {
            let sz = child_or_new(e, "w:pgSz", SECT_ORDER);
            sz.set_attr("w:w", twips(w));
            sz.set_attr("w:h", twips(h));
            if w > h {
                sz.set_attr("w:orient", "landscape");
            } else {
                sz.remove_attr("w:orient");
            }
        }
        if let Some([t, r, b, l]) = page.margin {
            let m = child_or_new(e, "w:pgMar", SECT_ORDER);
            for (k, v) in [("w:top", t), ("w:right", r), ("w:bottom", b), ("w:left", l)] {
                m.set_attr(k, twips(v));
            }
            for (k, v) in [("w:header", "708"), ("w:footer", "708"), ("w:gutter", "0")] {
                if m.attr(k).is_none() {
                    m.set_attr(k, v);
                }
            }
        }
    });
    true
}

/// The theme's fonts follow the stylesheet too: styles that name the theme's
/// heading or body font (title page, table of contents, …) then match.
pub(crate) fn write_theme_fonts(theme: &mut Element, sheet: &Stylesheet) -> bool {
    let body = sheet.rules.get(&Target::Body).and_then(Look::font);
    let head = sheet.resolved(Target::Heading(1)).and_then(|l| l.font()).or(body.clone());
    let mut changed = false;
    theme.visit_mut(&mut |e| {
        let f = if e.is("a:majorFont") {
            &head
        } else if e.is("a:minorFont") {
            &body
        } else {
            return;
        };
        if let (Some(f), Some(latin)) = (f, e.child_mut("a:latin")) {
            latin.set_attr("typeface", f.clone());
            changed = true;
        }
    });
    changed
}

fn on(e: &Element) -> bool {
    !matches!(e.attr("w:val"), Some("0" | "false" | "off" | "none"))
}

fn read_rpr(rpr: &Element, fonts: &(Option<String>, Option<String>), l: &mut Look) {
    for e in rpr.elements() {
        match e.name.as_str() {
            "w:rFonts" => {
                let theme = e.attr("w:asciiTheme").map(|t| if t.starts_with("major") { fonts.0.clone() } else { fonts.1.clone() });
                if let Some(f) = theme.flatten().or_else(|| e.attr("w:ascii").or(e.attr("w:hAnsi")).map(String::from)) {
                    l.family = Some(css_family(&f));
                }
            }
            "w:b" => l.bold = Some(on(e)),
            "w:i" => l.italic = Some(on(e)),
            "w:caps" => l.caps = Some(on(e)),
            "w:smallCaps" => l.small_caps = Some(on(e)),
            "w:u" => l.underline = Some(on(e)),
            "w:color" => {
                if let Some(c) = e.attr("w:val").filter(|c| *c != "auto") {
                    l.color = Some(c.to_ascii_uppercase());
                }
            }
            "w:sz" => l.size = e.attr("w:val").and_then(|v| v.parse::<f32>().ok()).map(|v| v / 2.0),
            "w:spacing" => l.letter = e.attr("w:val").and_then(|v| v.parse::<f32>().ok()).map(|v| v / 20.0),
            _ => {}
        }
    }
}

fn read_ppr(ppr: &Element, l: &mut Look) {
    let tw = |e: &Element, k: &str| e.attr(k).and_then(|v| v.parse::<f32>().ok()).map(|v| v / 20.0);
    for e in ppr.elements() {
        match e.name.as_str() {
            "w:spacing" => {
                l.before = tw(e, "w:before").or(l.before);
                l.after = tw(e, "w:after").or(l.after);
                if let Some(v) = e.attr("w:line").and_then(|v| v.parse::<f32>().ok()) {
                    l.line = Some(match e.attr("w:lineRule") {
                        Some("exact" | "atLeast") => Line::AtLeast(v / 20.0),
                        _ => Line::Multiple(v / 240.0),
                    });
                }
            }
            "w:ind" => {
                l.indent = tw(e, "w:left").or(tw(e, "w:start")).or(l.indent);
                if let Some(x) = tw(e, "w:firstLine") {
                    l.first_line = Some(x);
                } else if let Some(x) = tw(e, "w:hanging") {
                    l.first_line = Some(-x);
                }
            }
            "w:jc" => {
                l.align = e.attr("w:val").map(|v| match v {
                    "both" | "distribute" => "justify",
                    "center" => "center",
                    "right" | "end" => "right",
                    _ => "left",
                }.to_string())
            }
            "w:pBdr" => {
                l.rule = e.child("w:bottom").filter(|b| on(b)).map(|b| {
                    let w = b.attr("w:sz").and_then(|v| v.parse::<f32>().ok()).unwrap_or(4.0) / 8.0;
                    (w, b.attr("w:color").filter(|c| *c != "auto").unwrap_or("000000").to_ascii_uppercase())
                })
            }
            "w:pageBreakBefore" => l.page_break = Some(on(e)),
            "w:shd" => l.background = e.attr("w:fill").filter(|f| *f != "auto").map(|f| f.to_ascii_uppercase()),
            "w:numPr" => l.numbered = Some(e.child("w:numId").and_then(|n| n.attr("w:val")).is_some_and(|v| v != "0")),
            _ => {}
        }
    }
}

/// A document's look: its built-in styles (with what they inherit) and page.
pub fn read(styles: Option<&Document>, theme: Option<&Document>, body: &Element) -> Stylesheet {
    let mut sheet = Stylesheet::default();
    let fonts = theme_fonts(theme);
    if let Some(sect) = body.elements().filter(|e| e.is("w:sectPr")).last() {
        let tw = |e: &Element, k: &str| e.attr(k).and_then(|v| v.parse::<f32>().ok()).map(|v| v / 20.0);
        if let Some(sz) = sect.child("w:pgSz") {
            sheet.page.size = tw(sz, "w:w").zip(tw(sz, "w:h"));
        }
        if let Some(m) = sect.child("w:pgMar") {
            if let (Some(t), Some(r), Some(b), Some(l)) = (tw(m, "w:top"), tw(m, "w:right"), tw(m, "w:bottom"), tw(m, "w:left")) {
                sheet.page.margin = Some([t, r, b, l]);
            }
        }
    }
    let Some(styles) = styles else { return sheet };
    let root = &styles.root;
    let mut defaults = Look { size: Some(10.0), ..Look::default() };
    if let Some(r) = root.child_path(&["w:docDefaults", "w:rPrDefault", "w:rPr"]) {
        read_rpr(r, &fonts, &mut defaults);
    }
    if let Some(p) = root.child_path(&["w:docDefaults", "w:pPrDefault", "w:pPr"]) {
        read_ppr(p, &mut defaults);
    }
    let by_id = |id: &str| root.elements().find(|e| e.is("w:style") && e.attr("w:styleId") == Some(id));
    let targets = [Target::Body, Target::Title, Target::Subtitle, Target::Quote, Target::Caption, Target::Code, Target::Link]
        .into_iter()
        .chain((1..=6).map(Target::Heading));
    for t in targets {
        let style = find_style(root, t).and_then(|i| root.children[i].as_element());
        if style.is_none() && t != Target::Body {
            continue;
        }
        // The chain from the root style down to this one.
        let mut chain = Vec::new();
        let mut cur = style;
        while let Some(s) = cur {
            if chain.len() > 8 {
                break;
            }
            chain.push(s);
            cur = s.child("w:basedOn").and_then(|b| b.attr("w:val")).and_then(by_id);
        }
        let mut l = if t == Target::Link { Look::default() } else { defaults.clone() };
        for s in chain.iter().rev() {
            if let Some(p) = s.child("w:pPr") {
                read_ppr(p, &mut l);
            }
            if let Some(r) = s.child("w:rPr") {
                read_rpr(r, &fonts, &mut l);
            }
        }
        sheet.rules.insert(t, l);
    }
    // Only what differs from the body, as CSS would be written; numbering
    // only where it is on.
    let body = sheet.rules.get(&Target::Body).cloned().unwrap_or_default();
    for (t, l) in sheet.rules.iter_mut() {
        if l.numbered == Some(false) || !matches!(t, Target::Heading(_)) {
            l.numbered = None;
        }
        // Normal weight and style are what CSS assumes anyway.
        for b in [&mut l.bold, &mut l.italic, &mut l.caps, &mut l.small_caps] {
            if *b == Some(false) {
                *b = None;
            }
        }
        if *t == Target::Body || *t == Target::Link {
            continue;
        }
        if l.family == body.family {
            l.family = None;
        }
        if l.size == body.size {
            l.size = None;
        }
        if l.color == body.color {
            l.color = None;
        }
        if l.line == body.line {
            l.line = None;
        }
        if l.align == body.align {
            l.align = None;
        }
    }
    sheet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_subset() {
        let (s, w) = parse(
            "/* a theme */ @page { size: A4; margin: 2cm 2.5cm; }\n body { font: 11pt/1.3 \"Source Serif 4\", serif; color: #333; }\n h1, h2 { font-weight: 700; margin: 1em 0 0.5em; counter-increment: h; }\n h1 { font-size: 20pt; border-bottom: 1pt solid #ccc }\n table { color: red }",
        );
        assert_eq!(w.len(), 1, "{w:?}");
        let body = &s.rules[&Target::Body];
        assert_eq!((body.size, body.line, body.color.as_deref(), body.font().as_deref()), (Some(11.0), Some(Line::Multiple(1.3)), Some("333333"), Some("Source Serif 4")));
        let h1 = &s.rules[&Target::Heading(1)];
        // 1em of a 20pt heading.
        assert_eq!((h1.before, h1.after, h1.bold, h1.numbered), (Some(20.0), Some(10.0), Some(true), Some(true)));
        assert_eq!(h1.rule, Some((1.0, "CCCCCC".into())));
        assert_eq!(s.rules[&Target::Heading(2)].before, Some(11.0));
        let [t, r, _, _] = s.page.margin.unwrap();
        assert!((t - 2.0 * PT_PER_CM).abs() < 0.01 && (r - 2.5 * PT_PER_CM).abs() < 0.01);
    }

    #[test]
    fn builtin_themes_parse_cleanly() {
        for (name, _, css) in BUILTIN {
            let (s, w) = parse(css);
            assert!(w.is_empty(), "{name}: {w:?}");
            assert!(s.rules.contains_key(&Target::Body) && s.rules.contains_key(&Target::Heading(1)), "{name}");
        }
    }

    #[test]
    fn families_for_the_editor() {
        assert_eq!(css_family("Calibri"), "Calibri, Carlito, sans-serif");
        assert_eq!(css_family("Times New Roman"), "\"Times New Roman\", \"Liberation Serif\", Tinos, serif");
        assert_eq!(css_family("Consolas"), "Consolas, monospace");
    }
}
