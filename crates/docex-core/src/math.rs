//! Equations: Word's Office Math (OMML) <-> LaTeX.
//!
//! Covers what people actually write: fractions, scripts, roots, big
//! operators with limits, delimiters, matrices, functions, accents, bars,
//! text and font styles, Greek letters and common symbols. Anything else in
//! an existing equation makes `omml_to_latex` return `None`, and the equation
//! is kept as it is.

use crate::xml::{escape_attr, escape_text, Document, Element, Node};

/// (LaTeX command, character)
const SYMBOLS: &[(&str, &str)] = &[
    ("alpha", "α"), ("beta", "β"), ("gamma", "γ"), ("delta", "δ"), ("epsilon", "ϵ"), ("varepsilon", "ε"),
    ("zeta", "ζ"), ("eta", "η"), ("theta", "θ"), ("vartheta", "ϑ"), ("iota", "ι"), ("kappa", "κ"),
    ("lambda", "λ"), ("mu", "μ"), ("nu", "ν"), ("xi", "ξ"), ("pi", "π"), ("varpi", "ϖ"), ("rho", "ρ"),
    ("varrho", "ϱ"), ("sigma", "σ"), ("varsigma", "ς"), ("tau", "τ"), ("upsilon", "υ"), ("phi", "ϕ"),
    ("varphi", "φ"), ("chi", "χ"), ("psi", "ψ"), ("omega", "ω"), ("Gamma", "Γ"), ("Delta", "Δ"),
    ("Theta", "Θ"), ("Lambda", "Λ"), ("Xi", "Ξ"), ("Pi", "Π"), ("Sigma", "Σ"), ("Upsilon", "Υ"),
    ("Phi", "Φ"), ("Psi", "Ψ"), ("Omega", "Ω"),
    ("pm", "±"), ("mp", "∓"), ("times", "×"), ("div", "÷"), ("cdot", "⋅"), ("ast", "∗"), ("star", "⋆"),
    ("circ", "∘"), ("bullet", "∙"), ("oplus", "⊕"), ("otimes", "⊗"), ("leq", "≤"), ("le", "≤"),
    ("geq", "≥"), ("ge", "≥"), ("neq", "≠"), ("ne", "≠"), ("approx", "≈"), ("equiv", "≡"), ("sim", "∼"),
    ("simeq", "≃"), ("cong", "≅"), ("propto", "∝"), ("ll", "≪"), ("gg", "≫"), ("in", "∈"), ("notin", "∉"),
    ("ni", "∋"), ("subset", "⊂"), ("supset", "⊃"), ("subseteq", "⊆"), ("supseteq", "⊇"), ("cup", "∪"),
    ("cap", "∩"), ("setminus", "∖"), ("emptyset", "∅"), ("varnothing", "∅"), ("forall", "∀"),
    ("exists", "∃"), ("nexists", "∄"), ("neg", "¬"), ("land", "∧"), ("wedge", "∧"), ("lor", "∨"),
    ("vee", "∨"), ("infty", "∞"), ("partial", "∂"), ("nabla", "∇"), ("hbar", "ℏ"), ("ell", "ℓ"),
    ("Re", "ℜ"), ("Im", "ℑ"), ("aleph", "ℵ"), ("to", "→"), ("rightarrow", "→"), ("leftarrow", "←"),
    ("gets", "←"), ("leftrightarrow", "↔"), ("Rightarrow", "⇒"), ("Leftarrow", "⇐"),
    ("Leftrightarrow", "⇔"), ("iff", "⟺"), ("implies", "⟹"), ("mapsto", "↦"), ("uparrow", "↑"),
    ("downarrow", "↓"), ("ldots", "…"), ("dots", "…"), ("cdots", "⋯"), ("vdots", "⋮"), ("ddots", "⋱"),
    ("prime", "′"), ("degree", "°"), ("angle", "∠"), ("perp", "⊥"), ("parallel", "∥"), ("mid", "∣"),
    ("langle", "⟨"), ("rangle", "⟩"), ("lfloor", "⌊"), ("rfloor", "⌋"), ("lceil", "⌈"), ("rceil", "⌉"),
    ("vert", "|"), ("Vert", "‖"), ("triangle", "△"), ("square", "□"), ("checkmark", "✓"),
];

/// Big operators: (command, character, limits under/over by default)
const NARY: &[(&str, char, bool)] = &[
    ("sum", '∑', true), ("prod", '∏', true), ("coprod", '∐', true), ("int", '∫', false),
    ("iint", '∬', false), ("iiint", '∭', false), ("oint", '∮', false), ("bigcup", '⋃', true),
    ("bigcap", '⋂', true), ("bigvee", '⋁', true), ("bigwedge", '⋀', true), ("bigoplus", '⨁', true),
    ("bigotimes", '⨂', true),
];

const FUNCS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "log",
    "ln", "lg", "exp", "det", "dim", "ker", "deg", "gcd", "max", "min", "sup", "inf", "lim", "liminf",
    "limsup", "arg", "Pr", "hom",
];

/// (command, combining character)
const ACCENTS: &[(&str, char)] = &[
    ("hat", '\u{302}'), ("widehat", '\u{302}'), ("tilde", '\u{303}'), ("widetilde", '\u{303}'),
    ("bar", '\u{305}'), ("dot", '\u{307}'), ("ddot", '\u{308}'), ("vec", '\u{20d7}'), ("check", '\u{30c}'),
    ("breve", '\u{306}'), ("acute", '\u{301}'), ("grave", '\u{300}'),
];

const SPACES: &[(&str, &str)] = &[(",", "\u{2009}"), (":", "\u{2005}"), (";", "\u{2005}"), (" ", " "), ("quad", "\u{2003}"), ("qquad", "\u{2003}\u{2003}")];

const DELIMS: &[(&str, &str)] = &[
    ("(", "("), (")", ")"), ("[", "["), ("]", "]"), ("\\{", "{"), ("\\}", "}"), ("|", "|"), ("\\|", "‖"),
    ("\\langle", "⟨"), ("\\rangle", "⟩"), ("\\lfloor", "⌊"), ("\\rfloor", "⌋"), ("\\lceil", "⌈"),
    ("\\rceil", "⌉"), ("\\lvert", "|"), ("\\rvert", "|"), ("\\lVert", "‖"), ("\\rVert", "‖"), (".", ""),
];

const MATRICES: &[(&str, &str, &str)] = &[
    ("matrix", "", ""), ("pmatrix", "(", ")"), ("bmatrix", "[", "]"), ("Bmatrix", "{", "}"),
    ("vmatrix", "|", "|"), ("Vmatrix", "‖", "‖"), ("cases", "{", ""),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sty {
    Default,
    Plain,
    Bold,
    Italic,
}

#[derive(Clone, Debug)]
enum M {
    Chars(String, Sty, Option<&'static str>),
    Text(String),
    Group(Vec<M>),
    Frac(Vec<M>, Vec<M>, bool),
    Sqrt(Option<Vec<M>>, Vec<M>),
    Scripts(Box<M>, Option<Vec<M>>, Option<Vec<M>>),
    Nary(char, bool, Option<Vec<M>>, Option<Vec<M>>, Vec<M>),
    Delim(String, String, Vec<M>),
    Func(String, Option<Vec<M>>, Vec<M>),
    Accent(char, Vec<M>),
    Bar(bool, Vec<M>),
    Under(Vec<M>, Vec<M>),
    Over(Vec<M>, Vec<M>),
    Matrix(Vec<Vec<Vec<M>>>),
}

// ---------------------------------------------------------------------------
// LaTeX -> tree

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Cmd(String),
    Open,
    Close,
    Sup,
    Sub,
    Amp,
    Row,
    Ch(char),
    Ws,
}

fn tokenize(s: &str) -> Vec<Tok> {
    let cs: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        i += 1;
        match c {
            '\\' => {
                if i < cs.len() && cs[i].is_ascii_alphabetic() {
                    let st = i;
                    while i < cs.len() && cs[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    out.push(Tok::Cmd(cs[st..i].iter().collect()));
                } else if i < cs.len() && cs[i] == '\\' {
                    i += 1;
                    out.push(Tok::Row);
                } else if i < cs.len() {
                    out.push(Tok::Cmd(cs[i].to_string()));
                    i += 1;
                }
            }
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '^' => out.push(Tok::Sup),
            '_' => out.push(Tok::Sub),
            '&' => out.push(Tok::Amp),
            c if c.is_whitespace() => {
                if out.last() != Some(&Tok::Ws) {
                    out.push(Tok::Ws);
                }
            }
            c => out.push(Tok::Ch(c)),
        }
    }
    out
}

struct P {
    t: Vec<Tok>,
    i: usize,
}

impl P {
    fn skip_ws(&mut self) {
        while self.t.get(self.i) == Some(&Tok::Ws) {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<&Tok> {
        self.skip_ws();
        self.t.get(self.i)
    }

    fn next(&mut self) -> Option<Tok> {
        self.skip_ws();
        let t = self.t.get(self.i).cloned();
        self.i += 1;
        t
    }

    /// Elements until `}` / `&` / `\\` / `\right` / `\end` / end.
    fn row(&mut self) -> Vec<M> {
        let mut out = Vec::new();
        while let Some(t) = self.peek() {
            match t {
                Tok::Close | Tok::Amp | Tok::Row => break,
                Tok::Cmd(c) if c == "right" || c == "end" => break,
                _ => {}
            }
            match self.atom() {
                Some(a) => {
                    let a = self.scripts(a);
                    out.push(a);
                }
                None => break,
            }
        }
        merge_chars(out)
    }

    /// `{...}` or a single atom as an argument.
    fn arg(&mut self) -> Vec<M> {
        match self.peek() {
            Some(Tok::Open) => {
                self.next();
                let r = self.row();
                if self.peek() == Some(&Tok::Close) {
                    self.next();
                }
                r
            }
            Some(Tok::Close) | None => Vec::new(),
            _ => self.atom().map(|a| vec![a]).unwrap_or_default(),
        }
    }

    fn raw_arg(&mut self) -> String {
        if self.peek() != Some(&Tok::Open) {
            return match self.next() {
                Some(Tok::Ch(c)) => c.to_string(),
                _ => String::new(),
            };
        }
        self.next();
        let mut s = String::new();
        let mut depth = 0;
        while let Some(t) = {
            let t = self.t.get(self.i).cloned();
            self.i += 1;
            t
        } {
            match t {
                Tok::Open => {
                    depth += 1;
                    s.push('{');
                }
                Tok::Close if depth == 0 => break,
                Tok::Close => {
                    depth -= 1;
                    s.push('}');
                }
                Tok::Ch(c) => s.push(c),
                Tok::Ws => s.push(' '),
                Tok::Cmd(c) => {
                    s.push('\\');
                    s.push_str(&c);
                }
                _ => {}
            }
        }
        s
    }

    /// Text inside `\text{...}` keeps its spaces, so it is taken from the source.
    fn scripts(&mut self, base: M) -> M {
        let (mut sub, mut sup) = (None, None);
        loop {
            match self.peek() {
                Some(Tok::Sub) if sub.is_none() => {
                    self.next();
                    sub = Some(self.arg());
                }
                Some(Tok::Sup) if sup.is_none() => {
                    self.next();
                    sup = Some(self.arg());
                }
                Some(Tok::Ch('\'')) if sup.is_none() => {
                    self.next();
                    sup = Some(vec![M::Chars("′".into(), Sty::Default, None)]);
                }
                _ => break,
            }
        }
        match base {
            M::Nary(c, lim, None, None, _) => {
                let body = if matches!(self.peek(), None | Some(Tok::Close) | Some(Tok::Amp) | Some(Tok::Row)) {
                    Vec::new()
                } else {
                    self.atom().map(|a| vec![self.scripts(a)]).unwrap_or_default()
                };
                M::Nary(c, lim, sub, sup, body)
            }
            M::Func(name, None, _) => {
                let under = if name == "lim" || name == "max" || name == "min" || name == "sup" || name == "inf" { sub.take() } else { None };
                let name_m = match (sub, sup) {
                    (None, None) => None,
                    (s, p) => Some((s, p)),
                };
                let body = if matches!(self.peek(), None | Some(Tok::Close) | Some(Tok::Amp) | Some(Tok::Row)) {
                    Vec::new()
                } else {
                    self.atom().map(|a| vec![self.scripts(a)]).unwrap_or_default()
                };
                match name_m {
                    None => M::Func(name, under, body),
                    // \sin^2 x: scripts on the function name.
                    Some((s, p)) => M::Group(vec![M::Scripts(Box::new(M::Chars(name.clone(), Sty::Plain, None)), s, p), M::Group(body)]),
                }
            }
            b if sub.is_none() && sup.is_none() => b,
            b => M::Scripts(Box::new(b), sub, sup),
        }
    }

    fn atom(&mut self) -> Option<M> {
        let t = self.next()?;
        Some(match t {
            Tok::Ch(c) => M::Chars(c.to_string(), Sty::Default, None),
            Tok::Open => {
                let r = self.row();
                if self.peek() == Some(&Tok::Close) {
                    self.next();
                }
                M::Group(r)
            }
            Tok::Sup | Tok::Sub => {
                // A script with no base: attach to an empty base.
                self.i -= 1;
                M::Chars(String::new(), Sty::Default, None)
            }
            Tok::Close | Tok::Amp | Tok::Row | Tok::Ws => return None,
            Tok::Cmd(c) => self.command(&c),
        })
    }

    fn command(&mut self, c: &str) -> M {
        if let Some((_, s)) = SYMBOLS.iter().find(|(n, _)| *n == c) {
            return M::Chars(s.to_string(), Sty::Default, None);
        }
        if let Some((_, ch, lim)) = NARY.iter().find(|(n, _, _)| *n == c) {
            return M::Nary(*ch, *lim, None, None, Vec::new());
        }
        if FUNCS.contains(&c) {
            return M::Func(c.to_string(), None, Vec::new());
        }
        if let Some((_, ch)) = ACCENTS.iter().find(|(n, _)| *n == c) {
            return M::Accent(*ch, self.arg());
        }
        if let Some((_, s)) = SPACES.iter().find(|(n, _)| *n == c) {
            return M::Chars(s.to_string(), Sty::Default, None);
        }
        match c {
            "frac" | "dfrac" | "tfrac" => {
                let a = self.arg();
                M::Frac(a, self.arg(), true)
            }
            "binom" => {
                let a = self.arg();
                M::Delim("(".into(), ")".into(), vec![M::Frac(a, self.arg(), false)])
            }
            "sqrt" => {
                let deg = if self.peek() == Some(&Tok::Ch('[')) {
                    self.next();
                    let mut d = Vec::new();
                    while let Some(t) = self.peek() {
                        if *t == Tok::Ch(']') {
                            self.next();
                            break;
                        }
                        match self.atom() {
                            Some(a) => d.push(a),
                            None => break,
                        }
                    }
                    Some(merge_chars(d))
                } else {
                    None
                };
                M::Sqrt(deg, self.arg())
            }
            "left" => {
                let open = self.delim();
                let body = self.row();
                let close = if self.peek() == Some(&Tok::Cmd("right".into())) {
                    self.next();
                    self.delim()
                } else {
                    String::new()
                };
                M::Delim(open, close, body)
            }
            "overline" => M::Bar(true, self.arg()),
            "underline" => M::Bar(false, self.arg()),
            "underset" => {
                let lim = self.arg();
                M::Under(self.arg(), lim)
            }
            "overset" => {
                let lim = self.arg();
                M::Over(self.arg(), lim)
            }
            "text" | "textrm" | "mbox" | "textit" | "textbf" => M::Text(self.raw_arg()),
            "operatorname" => M::Func(self.raw_arg(), None, Vec::new()),
            "mathrm" => restyle(self.arg(), Sty::Plain, None),
            "mathbf" | "boldsymbol" => restyle(self.arg(), Sty::Bold, None),
            "mathit" => restyle(self.arg(), Sty::Italic, None),
            "mathbb" => restyle(self.arg(), Sty::Plain, Some("double-struck")),
            "mathcal" | "mathscr" => restyle(self.arg(), Sty::Plain, Some("script")),
            "mathfrak" => restyle(self.arg(), Sty::Plain, Some("fraktur")),
            "begin" => {
                let env = self.raw_arg();
                let mut rows = vec![vec![]];
                loop {
                    let cell = self.row();
                    rows.last_mut().unwrap().push(cell);
                    match self.next() {
                        Some(Tok::Amp) => {}
                        Some(Tok::Row) => rows.push(vec![]),
                        Some(Tok::Cmd(e)) if e == "end" => {
                            self.raw_arg();
                            break;
                        }
                        _ => break,
                    }
                }
                if rows.last().is_some_and(|r| r.len() == 1 && r[0].is_empty()) && rows.len() > 1 {
                    rows.pop();
                }
                let (open, close) = MATRICES.iter().find(|(n, _, _)| *n == env).map(|(_, o, c)| (*o, *c)).unwrap_or(("", ""));
                let m = M::Matrix(rows);
                if open.is_empty() && close.is_empty() { m } else { M::Delim(open.into(), close.into(), vec![m]) }
            }
            "{" | "}" | "%" | "#" | "&" | "_" | "$" => M::Chars(c.to_string(), Sty::Default, None),
            "|" => M::Chars("‖".into(), Sty::Default, None),
            "backslash" => M::Chars("\\".into(), Sty::Default, None),
            "displaystyle" | "textstyle" | "limits" | "nolimits" | "!" => M::Group(Vec::new()),
            other => M::Chars(format!("\\{other}"), Sty::Default, None),
        }
    }

    fn delim(&mut self) -> String {
        match self.next() {
            Some(Tok::Ch(c)) => {
                if c == '.' { String::new() } else { c.to_string() }
            }
            Some(Tok::Cmd(c)) => DELIMS
                .iter()
                .find(|(n, _)| n.strip_prefix('\\') == Some(c.as_str()))
                .map(|(_, ch)| ch.to_string())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
}

fn restyle(v: Vec<M>, sty: Sty, scr: Option<&'static str>) -> M {
    M::Group(
        v.into_iter()
            .map(|m| match m {
                M::Chars(s, _, _) => M::Chars(s, sty, scr),
                other => other,
            })
            .collect(),
    )
}

/// Join neighbouring plain letters/digits into one run, as Word does.
fn merge_chars(v: Vec<M>) -> Vec<M> {
    let mut out: Vec<M> = Vec::new();
    for m in v {
        if let (Some(M::Chars(a, sa, ca)), M::Chars(b, sb, cb)) = (out.last_mut(), &m) {
            let word = |s: &str| s.chars().all(|c| c.is_alphanumeric() || c == '.');
            if sa == sb && ca == cb && word(a) && word(b) && !a.is_empty() && !b.is_empty() && !a.starts_with('\\') && !b.starts_with('\\') {
                a.push_str(b);
                continue;
            }
        }
        out.push(m);
    }
    out
}

// ---------------------------------------------------------------------------
// tree -> OMML

fn omml(v: &[M], out: &mut String) {
    for m in v {
        one(m, out);
    }
}

fn wrap(tag: &str, v: &[M], out: &mut String) {
    out.push_str(&format!("<m:{tag}>"));
    omml(v, out);
    out.push_str(&format!("</m:{tag}>"));
}

fn val(tag: &str, v: &str) -> String {
    format!("<m:{tag} m:val=\"{}\"/>", escape_attr(v))
}

fn run(text: &str, sty: Sty, scr: Option<&str>, out: &mut String) {
    out.push_str("<m:r>");
    if sty != Sty::Default || scr.is_some() {
        out.push_str("<m:rPr>");
        if let Some(s) = scr {
            out.push_str(&val("scr", s));
        }
        match sty {
            Sty::Plain => out.push_str(&val("sty", "p")),
            Sty::Bold => out.push_str(&val("sty", "b")),
            Sty::Italic => out.push_str(&val("sty", "i")),
            Sty::Default => {}
        }
        out.push_str("</m:rPr>");
    }
    out.push_str(&format!("<m:t xml:space=\"preserve\">{}</m:t></m:r>", escape_text(text)));
}

fn one(m: &M, out: &mut String) {
    match m {
        M::Chars(s, sty, scr) => {
            if !s.is_empty() {
                run(s, *sty, *scr, out)
            }
        }
        M::Text(s) => {
            out.push_str(&format!("<m:r><m:rPr><m:nor/></m:rPr><m:t xml:space=\"preserve\">{}</m:t></m:r>", escape_text(s)));
        }
        M::Group(v) => omml(v, out),
        M::Frac(a, b, bar) => {
            out.push_str("<m:f>");
            if !bar {
                out.push_str(&format!("<m:fPr>{}</m:fPr>", val("type", "noBar")));
            }
            wrap("num", a, out);
            wrap("den", b, out);
            out.push_str("</m:f>");
        }
        M::Sqrt(deg, e) => {
            out.push_str("<m:rad>");
            match deg {
                Some(d) => {
                    wrap("deg", d, out);
                }
                None => out.push_str(&format!("<m:radPr>{}</m:radPr><m:deg/>", val("degHide", "1"))),
            }
            wrap("e", e, out);
            out.push_str("</m:rad>");
        }
        M::Scripts(base, sub, sup) => {
            let base = std::slice::from_ref(base.as_ref());
            match (sub, sup) {
                (Some(s), Some(p)) => {
                    out.push_str("<m:sSubSup>");
                    wrap("e", base, out);
                    wrap("sub", s, out);
                    wrap("sup", p, out);
                    out.push_str("</m:sSubSup>");
                }
                (Some(s), None) => {
                    out.push_str("<m:sSub>");
                    wrap("e", base, out);
                    wrap("sub", s, out);
                    out.push_str("</m:sSub>");
                }
                (None, Some(p)) => {
                    out.push_str("<m:sSup>");
                    wrap("e", base, out);
                    wrap("sup", p, out);
                    out.push_str("</m:sSup>");
                }
                (None, None) => omml(base, out),
            }
        }
        M::Nary(c, lim, sub, sup, body) => {
            out.push_str("<m:nary><m:naryPr>");
            out.push_str(&val("chr", &c.to_string()));
            out.push_str(&val("limLoc", if *lim { "undOvr" } else { "subSup" }));
            if sub.is_none() {
                out.push_str(&val("subHide", "1"));
            }
            if sup.is_none() {
                out.push_str(&val("supHide", "1"));
            }
            out.push_str("</m:naryPr>");
            wrap("sub", sub.as_deref().unwrap_or(&[]), out);
            wrap("sup", sup.as_deref().unwrap_or(&[]), out);
            wrap("e", body, out);
            out.push_str("</m:nary>");
        }
        M::Delim(o, c, body) => {
            out.push_str(&format!("<m:d><m:dPr>{}{}</m:dPr>", val("begChr", o), val("endChr", c)));
            wrap("e", body, out);
            out.push_str("</m:d>");
        }
        M::Func(name, under, body) => {
            out.push_str("<m:func><m:fName>");
            match under {
                Some(u) => {
                    out.push_str("<m:limLow><m:e>");
                    run(name, Sty::Plain, None, out);
                    out.push_str("</m:e>");
                    wrap("lim", u, out);
                    out.push_str("</m:limLow>");
                }
                None => run(name, Sty::Plain, None, out),
            }
            out.push_str("</m:fName>");
            wrap("e", body, out);
            out.push_str("</m:func>");
        }
        M::Accent(c, e) => {
            out.push_str(&format!("<m:acc><m:accPr>{}</m:accPr>", val("chr", &c.to_string())));
            wrap("e", e, out);
            out.push_str("</m:acc>");
        }
        M::Bar(top, e) => {
            out.push_str(&format!("<m:bar><m:barPr>{}</m:barPr>", val("pos", if *top { "top" } else { "bot" })));
            wrap("e", e, out);
            out.push_str("</m:bar>");
        }
        M::Under(e, lim) => {
            out.push_str("<m:limLow>");
            wrap("e", e, out);
            wrap("lim", lim, out);
            out.push_str("</m:limLow>");
        }
        M::Over(e, lim) => {
            out.push_str("<m:limUpp>");
            wrap("e", e, out);
            wrap("lim", lim, out);
            out.push_str("</m:limUpp>");
        }
        M::Matrix(rows) => {
            out.push_str("<m:m>");
            for r in rows {
                out.push_str("<m:mr>");
                for c in r {
                    wrap("e", c, out);
                }
                out.push_str("</m:mr>");
            }
            out.push_str("</m:m>");
        }
    }
}

/// A Word equation (`m:oMath`, or `m:oMathPara` for display math) from LaTeX.
pub fn latex_to_omml(latex: &str, display: bool) -> Element {
    let mut p = P { t: tokenize(latex), i: 0 };
    let mut v = p.row();
    // Stray closing braces etc.: keep going so no text is lost.
    while p.i < p.t.len() {
        p.next();
        v.extend(p.row());
    }
    let mut s = String::from("<m:oMath xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\">");
    omml(&v, &mut s);
    s.push_str("</m:oMath>");
    let mut math = Document::parse(&s).expect("generated OMML parses").root;
    math.remove_attr("xmlns:m");
    if display {
        Element::new("m:oMathPara").with_child(math)
    } else {
        math
    }
}

// ---------------------------------------------------------------------------
// OMML -> LaTeX

fn prop<'a>(e: &'a Element, pr: &str, name: &str) -> Option<&'a str> {
    e.child(pr)?.child(name)?.attr("m:val")
}

fn latex_of_char(c: char, out: &mut String) {
    let s = c.to_string();
    if let Some((n, _)) = SYMBOLS.iter().find(|(_, ch)| *ch == s) {
        out.push('\\');
        out.push_str(n);
        out.push(' ');
        return;
    }
    match c {
        '{' | '}' | '%' | '#' | '&' | '_' | '$' => {
            out.push('\\');
            out.push(c);
        }
        '\\' => out.push_str("\\backslash "),
        '^' => out.push_str("\\hat{}"),
        '\u{2009}' => out.push_str("\\,"),
        '\u{2005}' => out.push_str("\\;"),
        '\u{2003}' => out.push_str("\\quad "),
        ' ' => {}
        c => out.push(c),
    }
}

fn children(e: &Element, out: &mut String) -> Option<()> {
    for c in e.elements() {
        if c.name.ends_with("Pr") || c.is("m:ctrlPr") {
            continue;
        }
        elem(c, out)?;
    }
    Some(())
}

fn group(e: Option<&Element>) -> Option<String> {
    let mut s = String::new();
    if let Some(e) = e {
        children(e, &mut s)?;
    }
    Some(s.trim_end().to_string())
}

fn braced(e: Option<&Element>) -> Option<String> {
    Some(format!("{{{}}}", group(e)?))
}

/// A script or base argument: bare when it is one character or one command
/// (`x^2`, `a_\infty`), braced otherwise (`x^{10}`).
fn script_arg(e: Option<&Element>) -> Option<String> {
    let g = group(e)?;
    let single_char = g.chars().count() == 1 && !g.starts_with(['{', '}', '^', '_', '&']);
    let single_cmd = g.starts_with('\\') && g.len() > 1 && g[1..].chars().all(|c| c.is_ascii_alphabetic());
    Some(if single_char || single_cmd { g } else { format!("{{{g}}}") })
}

/// The operand of a big operator or function: bare (after a space) when it
/// is a single atom the parser takes back as the operand, braced otherwise.
fn operand(e: Option<&Element>) -> Option<String> {
    let parts: Vec<&Element> = e.map(|e| e.elements().filter(|c| !c.name.ends_with("Pr")).collect()).unwrap_or_default();
    let atomic = match parts.as_slice() {
        [r] if r.is("m:r") => run_text(r).chars().count() == 1 && r.child("m:rPr").is_none(),
        [x] => !x.is("m:d"),
        _ => false,
    };
    let g = group(e)?;
    Some(if atomic && !g.is_empty() { format!(" {g}") } else { format!("{{{g}}}") })
}

fn run_text(r: &Element) -> String {
    r.elements().filter(|t| t.is("m:t")).map(|t| t.text_content()).collect()
}

fn elem(e: &Element, out: &mut String) -> Option<()> {
    match e.name.as_str() {
        "m:r" => {
            let text = run_text(e);
            let rpr = e.child("m:rPr");
            if rpr.is_some_and(|p| p.child("m:nor").is_some()) {
                out.push_str(&format!("\\text{{{}}}", text.replace(['{', '}'], "")));
                return Some(());
            }
            let sty = rpr.and_then(|p| p.child("m:sty")).and_then(|s| s.attr("m:val"));
            let scr = rpr.and_then(|p| p.child("m:scr")).and_then(|s| s.attr("m:val"));
            let mut inner = String::new();
            for c in text.chars() {
                latex_of_char(c, &mut inner);
            }
            let cmd = match (scr, sty) {
                (Some("double-struck"), _) => Some("mathbb"),
                (Some("script"), _) => Some("mathcal"),
                (Some("fraktur"), _) => Some("mathfrak"),
                (Some(_), _) => return None,
                (None, Some("p")) if text.chars().any(|c| c.is_alphabetic()) => Some("mathrm"),
                (None, Some("b")) | (None, Some("bi")) => Some("mathbf"),
                _ => None,
            };
            match cmd {
                Some(c) => out.push_str(&format!("\\{c}{{{}}}", inner.trim_end())),
                None => out.push_str(&inner),
            }
        }
        "m:f" => {
            let num = braced(e.child("m:num"))?;
            let den = braced(e.child("m:den"))?;
            if prop(e, "m:fPr", "m:type") == Some("noBar") {
                return None;
            }
            out.push_str(&format!("\\frac{num}{den}"));
        }
        "m:rad" => {
            let hide = matches!(prop(e, "m:radPr", "m:degHide"), Some("1") | Some("on") | Some("true"));
            let deg = group(e.child("m:deg"))?;
            if hide || deg.is_empty() {
                out.push_str(&format!("\\sqrt{}", braced(e.child("m:e"))?));
            } else {
                out.push_str(&format!("\\sqrt[{deg}]{}", braced(e.child("m:e"))?));
            }
        }
        "m:sSub" | "m:sSup" | "m:sSubSup" => {
            out.push_str(&script_arg(e.child("m:e"))?);
            if let Some(s) = e.child("m:sub") {
                out.push_str(&format!("_{}", script_arg(Some(s))?));
            }
            if let Some(s) = e.child("m:sup") {
                out.push_str(&format!("^{}", script_arg(Some(s))?));
            }
        }
        "m:nary" => {
            let ch = prop(e, "m:naryPr", "m:chr").and_then(|c| c.chars().next()).unwrap_or('∫');
            let (cmd, _, _) = NARY.iter().find(|(_, c, _)| *c == ch)?;
            out.push('\\');
            out.push_str(cmd);
            let hidden = |n: &str| matches!(prop(e, "m:naryPr", n), Some("1") | Some("on") | Some("true"));
            let sub = group(e.child("m:sub"))?;
            let sup = group(e.child("m:sup"))?;
            if !hidden("m:subHide") && !sub.is_empty() {
                out.push_str(&format!("_{}", script_arg(e.child("m:sub"))?));
            }
            if !hidden("m:supHide") && !sup.is_empty() {
                out.push_str(&format!("^{}", script_arg(e.child("m:sup"))?));
            }
            out.push_str(&operand(e.child("m:e"))?);
        }
        "m:d" => {
            let es: Vec<&Element> = e.elements().filter(|c| c.is("m:e")).collect();
            if es.len() != 1 {
                return None;
            }
            let beg = prop(e, "m:dPr", "m:begChr").unwrap_or("(");
            let end = prop(e, "m:dPr", "m:endChr").unwrap_or(")");
            // A lone matrix: pmatrix & co.
            let inner: Vec<&Element> = es[0].elements().filter(|c| !c.name.ends_with("Pr")).collect();
            if inner.len() == 1 && inner[0].is("m:m") {
                if let Some((env, _, _)) = MATRICES.iter().find(|(_, o, c)| *o == beg && *c == end) {
                    out.push_str(&matrix(inner[0], env)?);
                    return Some(());
                }
            }
            if beg == "(" && end == ")" && inner.len() == 1 && inner[0].is("m:f") && prop(inner[0], "m:fPr", "m:type") == Some("noBar") {
                out.push_str(&format!("\\binom{}{}", braced(inner[0].child("m:num"))?, braced(inner[0].child("m:den"))?));
                return Some(());
            }
            let d = |c: &str| -> Option<String> {
                if c.is_empty() {
                    return Some(".".into());
                }
                DELIMS.iter().find(|(_, ch)| *ch == c).map(|(n, _)| n.to_string())
            };
            out.push_str(&format!("\\left{} {} \\right{}", d(beg)?, group(Some(es[0]))?, d(end)?));
        }
        "m:func" => {
            let fname = e.child("m:fName")?;
            let body = operand(e.child("m:e"))?;
            let parts: Vec<&Element> = fname.elements().collect();
            match parts.as_slice() {
                [r] if r.is("m:r") => {
                    let name = run_text(r);
                    if FUNCS.contains(&name.as_str()) {
                        out.push_str(&format!("\\{name}{body}"));
                    } else {
                        out.push_str(&format!("\\operatorname{{{name}}}{body}"));
                    }
                }
                [l] if l.is("m:limLow") => {
                    let name = l.child("m:e").map(|n| n.elements().filter(|r| r.is("m:r")).map(run_text).collect::<String>()).unwrap_or_default();
                    if !FUNCS.contains(&name.as_str()) {
                        return None;
                    }
                    out.push_str(&format!("\\{name}_{}{body}", script_arg(l.child("m:lim"))?));
                }
                _ => return None,
            }
        }
        "m:acc" => {
            let ch = prop(e, "m:accPr", "m:chr").and_then(|c| c.chars().next()).unwrap_or('\u{302}');
            let (cmd, _) = ACCENTS.iter().find(|(_, c)| *c == ch)?;
            out.push_str(&format!("\\{cmd}{}", braced(e.child("m:e"))?));
        }
        "m:bar" => {
            let top = prop(e, "m:barPr", "m:pos") == Some("top");
            out.push_str(&format!("\\{}{}", if top { "overline" } else { "underline" }, braced(e.child("m:e"))?));
        }
        "m:limLow" => out.push_str(&format!("\\underset{}{}", braced(e.child("m:lim"))?, braced(e.child("m:e"))?)),
        "m:limUpp" => out.push_str(&format!("\\overset{}{}", braced(e.child("m:lim"))?, braced(e.child("m:e"))?)),
        "m:m" => out.push_str(&matrix(e, "matrix")?),
        "m:box" | "m:e" => children(e, out)?,
        "w:bookmarkStart" | "w:bookmarkEnd" | "w:proofErr" => {}
        _ => return None,
    }
    Some(())
}

fn matrix(m: &Element, env: &str) -> Option<String> {
    let mut rows = Vec::new();
    for r in m.elements().filter(|r| r.is("m:mr")) {
        let cells: Option<Vec<String>> = r.elements().filter(|c| c.is("m:e")).map(|c| group(Some(c))).collect();
        rows.push(cells?.join(" & "));
    }
    Some(format!("\\begin{{{env}}} {} \\end{{{env}}}", rows.join(" \\\\ ")))
}

/// LaTeX for a Word equation, or `None` if it uses something not covered.
pub fn omml_to_latex(e: &Element) -> Option<String> {
    let math = if e.is("m:oMathPara") {
        let ms: Vec<&Element> = e.elements().filter(|c| c.is("m:oMath")).collect();
        if ms.len() != 1 {
            return None;
        }
        ms[0]
    } else {
        e
    };
    let mut s = String::new();
    children(math, &mut s)?;
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() || s.contains('$') {
        return None;
    }
    Some(s)
}

/// Whether `n` is an equation node.
pub fn is_math(n: &Node) -> bool {
    matches!(n, Node::Element(e) if e.is("m:oMath") || e.is("m:oMathPara"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(latex: &str) -> String {
        let el = latex_to_omml(latex, false);
        omml_to_latex(&el).unwrap_or_else(|| panic!("no latex back for {latex}: {}", el.to_xml()))
    }

    #[test]
    fn roundtrips() {
        for (src, want) in [
            ("x^2 + y^2 = z^2", "x^2+y^2=z^2"),
            ("\\frac{a}{b}", "\\frac{a}{b}"),
            ("\\sqrt{x}", "\\sqrt{x}"),
            ("\\sqrt[3]{x}", "\\sqrt[3]{x}"),
            ("\\sum_{i=1}^{n} x_i", "\\sum_{i=1}^n x_i"),
            ("\\int_0^1 f(x)\\,dx", "\\int_0^1 f(x)\\,dx"),
            ("\\alpha + \\beta \\leq \\gamma", "\\alpha +\\beta \\leq \\gamma"),
            ("\\left( \\frac{1}{2} \\right)", "\\left( \\frac{1}{2} \\right)"),
            ("\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}", "\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}"),
            ("\\sin x + \\lim_{n \\to \\infty} a_n", "\\sin x+\\lim_{n\\to \\infty} a_n"),
            ("\\hat{x} \\vec{v} \\overline{z}", "\\hat{x}\\vec{v}\\overline{z}"),
            ("\\text{if } x > 0", "\\text{if }x>0"),
            ("\\mathbb{R}^n", "{\\mathbb{R}}^n"),
            ("\\binom{n}{k}", "\\binom{n}{k}"),
            ("E = mc^2", "E=mc^2"),
        ] {
            assert_eq!(rt(src), want, "{src}");
        }
    }

    #[test]
    fn stable() {
        // LaTeX produced from OMML converts back to the same OMML.
        for src in ["\\sum_{i=1}^{n} x_i^2", "\\frac{\\partial f}{\\partial x}", "\\left[ a, b \\right)", "\\mathrm{d}x"] {
            let a = latex_to_omml(src, false);
            let l = omml_to_latex(&a).unwrap();
            let b = latex_to_omml(&l, false);
            assert_eq!(a.to_xml(), b.to_xml(), "{src} -> {l}");
            assert_eq!(omml_to_latex(&b).unwrap(), l);
        }
    }

    #[test]
    fn unknown_is_none() {
        let d = Document::parse("<m:oMath xmlns:m=\"x\"><m:groupChr><m:e><m:r><m:t>x</m:t></m:r></m:e></m:groupChr></m:oMath>").unwrap();
        assert!(omml_to_latex(&d.root).is_none());
    }
}
