//! Word's automatic numbering ("3.2", "(b)", "iv.") worked out from
//! `numbering.xml`, for display. docex never writes these labels: Word
//! computes them from the same definitions.

use crate::styles::Styles;
use crate::xml::{Document, Element};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default)]
struct Lvl {
    start: u32,
    fmt: String,
    text: String,
    /// Legal numbering: all levels shown as decimal ("1.2.3" under "Article I").
    legal: bool,
    /// `w:lvlRestart w:val="0"`: never restarts.
    no_restart: bool,
    /// Paragraph style tied to this level (`w:pStyle`), e.g. Heading2.
    style: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct Abstract {
    levels: HashMap<u8, Lvl>,
    /// `w:numStyleLink`: the definition lives with this numbering style.
    style_link: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct Num {
    abs: String,
    /// Per level: a new start value and/or a replacement level definition.
    overrides: HashMap<u8, (Option<u32>, Option<Lvl>)>,
}

#[derive(Debug, Default)]
pub struct Numbering {
    abs: HashMap<String, Abstract>,
    nums: HashMap<String, Num>,
    /// numStyleLink target style -> numId.
    style_nums: HashMap<String, String>,
}

fn val<'a>(e: &'a Element, name: &str) -> Option<&'a str> {
    e.child(name).and_then(|c| c.attr("w:val"))
}

fn lvl(e: &Element) -> Lvl {
    Lvl {
        start: val(e, "w:start").and_then(|v| v.parse().ok()).unwrap_or(0),
        fmt: val(e, "w:numFmt").unwrap_or("decimal").to_string(),
        text: val(e, "w:lvlText").unwrap_or("").to_string(),
        legal: e.child("w:isLgl").is_some_and(|l| !matches!(l.attr("w:val"), Some("0" | "false"))),
        no_restart: val(e, "w:lvlRestart") == Some("0"),
        style: val(e, "w:pStyle").map(String::from),
    }
}

fn ilvl(e: &Element) -> u8 {
    e.attr("w:ilvl").and_then(|v| v.parse().ok()).unwrap_or(0)
}

impl Numbering {
    pub fn load(doc: Option<&Document>, styles: &Styles) -> Numbering {
        let mut n = Numbering::default();
        let Some(doc) = doc else { return n };
        for a in doc.root.elements().filter(|e| e.is("w:abstractNum")) {
            let levels = a.elements().filter(|e| e.is("w:lvl")).map(|l| (ilvl(l), lvl(l))).collect();
            let style_link = val(a, "w:numStyleLink").map(String::from);
            n.abs.insert(a.attr("w:abstractNumId").unwrap_or("").to_string(), Abstract { levels, style_link });
        }
        for e in doc.root.elements().filter(|e| e.is("w:num")) {
            let mut num = Num { abs: val(e, "w:abstractNumId").unwrap_or("").to_string(), overrides: HashMap::new() };
            for o in e.elements().filter(|o| o.is("w:lvlOverride")) {
                let start = val(o, "w:startOverride").and_then(|v| v.parse().ok());
                num.overrides.insert(ilvl(o), (start, o.child("w:lvl").map(lvl)));
            }
            n.nums.insert(e.attr("w:numId").unwrap_or("").to_string(), num);
        }
        for st in styles.by_id.values() {
            if let Some(id) = &st.num_id {
                n.style_nums.insert(st.id.clone(), id.clone());
            }
        }
        n
    }

    /// The abstract definition a num uses, following numbering-style links.
    fn abstract_of(&self, num_id: &str) -> Option<(&str, &Abstract)> {
        let mut aid = self.nums.get(num_id)?.abs.as_str();
        for _ in 0..4 {
            let a = self.abs.get(aid)?;
            let Some(link) = &a.style_link else { return Some((aid, a)) };
            let next = self.style_nums.get(link).and_then(|n| self.nums.get(n))?;
            if next.abs == aid {
                return Some((aid, a));
            }
            aid = next.abs.as_str();
        }
        None
    }

    fn level(&self, num_id: &str, l: u8) -> Option<Lvl> {
        if let Some((_, Some(lv))) = self.nums.get(num_id).and_then(|n| n.overrides.get(&l)) {
            return Some(lv.clone());
        }
        self.abstract_of(num_id)?.1.levels.get(&l).cloned()
    }

    /// Level of a list tied to paragraph style `style` (headings numbered
    /// through their style often give only the numId).
    pub fn level_of_style(&self, num_id: &str, style: &str) -> Option<u8> {
        let (_, a) = self.abstract_of(num_id)?;
        a.levels.iter().find(|(_, l)| l.style.as_deref() == Some(style)).map(|(k, _)| *k)
    }
}

/// Counts through the paragraphs of a document in order.
#[derive(Default)]
pub struct Counter {
    /// abstract id -> current value per level.
    values: HashMap<String, [Option<u32>; 9]>,
    seen: HashSet<String>,
}

impl Counter {
    /// The label of the next paragraph numbered by `num_id` at level `l`;
    /// `None` for bullets and unnumbered levels.
    pub fn next(&mut self, n: &Numbering, num_id: &str, l: u8) -> Option<String> {
        let l = l.min(8);
        let (aid, _) = n.abstract_of(num_id)?;
        let aid = aid.to_string();
        let this = n.level(num_id, l)?;
        let num = n.nums.get(num_id)?;
        let st = self.values.entry(aid.clone()).or_default();
        // A list instance with a start override restarts where it is first used.
        if self.seen.insert(num_id.to_string()) {
            for (k, (start, _)) in &num.overrides {
                if start.is_some() && (*k as usize) < 9 {
                    st[*k as usize] = None;
                }
            }
        }
        let start = |k: u8| -> u32 {
            num.overrides.get(&k).and_then(|o| o.0).or_else(|| n.level(num_id, k).map(|v| v.start)).unwrap_or(0)
        };
        st[l as usize] = Some(st[l as usize].map(|v| v + 1).unwrap_or_else(|| start(l)));
        for k in l + 1..9 {
            if !n.level(num_id, k).is_some_and(|v| v.no_restart) {
                st[k as usize] = None;
            }
        }
        if this.fmt == "bullet" {
            return None;
        }
        let mut out = String::new();
        let mut chars = this.text.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, chars.peek().and_then(|d| d.to_digit(10))) {
                ('%', Some(d)) if (1..=9).contains(&d) => {
                    chars.next();
                    let k = (d - 1) as u8;
                    let v = st[k as usize].unwrap_or_else(|| start(k));
                    let fmt = if this.legal && k != l { "decimal".to_string() } else { n.level(num_id, k).map(|x| x.fmt).unwrap_or_default() };
                    out.push_str(&format_num(v, &fmt));
                }
                _ => out.push(c),
            }
        }
        let out = out.trim().to_string();
        (!out.is_empty()).then_some(out)
    }
}

/// A number in one of Word's formats (`w:numFmt`).
pub fn format_num(v: u32, fmt: &str) -> String {
    match fmt {
        "none" => String::new(),
        "decimalZero" => format!("{v:02}"),
        "lowerLetter" => letters(v).to_lowercase(),
        "upperLetter" => letters(v),
        "lowerRoman" => roman(v).to_lowercase(),
        "upperRoman" => roman(v),
        "ordinal" => {
            let suffix = match (v % 10, v % 100) {
                (_, 11..=13) => "th",
                (1, _) => "st",
                (2, _) => "nd",
                (3, _) => "rd",
                _ => "th",
            };
            format!("{v}{suffix}")
        }
        "cardinalText" => words(v),
        "ordinalText" => ordinal_words(v),
        _ => v.to_string(),
    }
}

/// a..z, then aa..zz (Word repeats the letter).
fn letters(v: u32) -> String {
    if v == 0 {
        return String::new();
    }
    let c = (b'A' + ((v - 1) % 26) as u8) as char;
    c.to_string().repeat(((v - 1) / 26 + 1) as usize)
}

fn roman(mut v: u32) -> String {
    const T: [(u32, &str); 13] = [
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"), (50, "L"),
        (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
    ];
    let mut s = String::new();
    for (n, r) in T {
        while v >= n {
            s.push_str(r);
            v -= n;
        }
    }
    s
}

const ONES: [&str; 20] = [
    "Zero", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten", "Eleven", "Twelve",
    "Thirteen", "Fourteen", "Fifteen", "Sixteen", "Seventeen", "Eighteen", "Nineteen",
];
const TENS: [&str; 10] = ["", "", "Twenty", "Thirty", "Forty", "Fifty", "Sixty", "Seventy", "Eighty", "Ninety"];

fn words(v: u32) -> String {
    match v {
        0..=19 => ONES[v as usize].into(),
        20..=99 => {
            let t = TENS[(v / 10) as usize];
            if v % 10 == 0 { t.into() } else { format!("{t}-{}", ONES[(v % 10) as usize].to_lowercase()) }
        }
        _ => v.to_string(),
    }
}

fn ordinal_words(v: u32) -> String {
    let w = words(v);
    if v > 99 {
        return format_num(v, "ordinal");
    }
    for (a, b) in [("One", "First"), ("one", "first"), ("Two", "Second"), ("two", "second"), ("Three", "Third"), ("three", "third"), ("Five", "Fifth"), ("five", "fifth"), ("Eight", "Eighth"), ("eight", "eighth"), ("Nine", "Ninth"), ("nine", "ninth"), ("Twelve", "Twelfth")] {
        if let Some(stem) = w.strip_suffix(a) {
            return format!("{stem}{b}");
        }
    }
    if let Some(stem) = w.strip_suffix('y') {
        return format!("{stem}ieth");
    }
    format!("{w}th")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(format_num(4, "lowerRoman"), "iv");
        assert_eq!(format_num(1994, "upperRoman"), "MCMXCIV");
        assert_eq!(format_num(28, "lowerLetter"), "bb");
        assert_eq!(format_num(12, "ordinal"), "12th");
        assert_eq!(format_num(22, "ordinal"), "22nd");
        assert_eq!(format_num(21, "cardinalText"), "Twenty-one");
        assert_eq!(format_num(3, "ordinalText"), "Third");
        assert_eq!(format_num(40, "ordinalText"), "Fortieth");
        assert_eq!(format_num(5, "decimalZero"), "05");
    }
}
