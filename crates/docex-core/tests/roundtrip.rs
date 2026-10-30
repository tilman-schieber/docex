use docex_core::render::View;
use docex_core::{ApplyOpts, Docx};
use std::io::{Read, Write};

/// A docx whose body is `body`, built on the blank template.
fn docx(body: &str) -> Docx {
    docx_with(body, None, "")
}

/// Like `docx`, with another `numbering.xml` and extra styles.
fn docx_with(body: &str, numbering: Option<&str>, styles: &str) -> Docx {
    docx_parts(body, numbering, styles, &[])
}

/// Like `docx_with`, plus extra parts `(name, relationship id, type, xml)`
/// related to the main document.
fn docx_parts(body: &str, numbering: Option<&str>, styles: &str, parts: &[(&str, &str, &str, &str)]) -> Docx {
    let blank = docex_core::template::blank("2026-01-01T00:00:00Z").unwrap();
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(blank)).unwrap();
    let mut zout = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut data = String::new();
        f.read_to_string(&mut data).unwrap();
        if name == "word/document.xml" {
            data = data.replace("<w:body>", &format!("<w:body>{body}"));
        }
        if let (Some(n), "word/numbering.xml") = (numbering, name.as_str()) {
            data = n.to_string();
        }
        if name == "word/styles.xml" {
            data = data.replace("</w:styles>", &format!("{styles}</w:styles>"));
        }
        if name == "word/_rels/document.xml.rels" {
            for (part, id, ty, _) in parts {
                let target = part.trim_start_matches("word/");
                data = data.replace("</Relationships>", &format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{ty}" Target="{target}"/></Relationships>"#));
            }
        }
        zout.start_file(name, opts).unwrap();
        zout.write_all(data.as_bytes()).unwrap();
    }
    for (part, _, _, xml) in parts {
        zout.start_file(*part, opts).unwrap();
        zout.write_all(xml.as_bytes()).unwrap();
    }
    Docx::from_bytes(zout.finish().unwrap().into_inner()).unwrap()
}

/// Markdown without WORDY's red colour (for checks on the text).
fn uncolored(md: &str) -> String {
    md.replace(r#"<span style="color:#FF0000">"#, "").replace("</span>", "")
}

fn opts(tracked: bool) -> ApplyOpts {
    ApplyOpts { author: "Me".into(), date: "2026-10-29T00:00:00Z".into(), tracked, base_dir: None, keep_opaque: false }
}

fn reopen(d: &mut Docx) -> Docx {
    Docx::from_bytes(d.to_bytes().unwrap()).unwrap()
}

const WORDY: &str = r#"<w:p w14:paraId="1A2B3C4D" w:rsidR="00A1"><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Contract</w:t></w:r></w:p><w:p><w:r><w:rPr><w:color w:val="FF0000"/><w:sz w:val="28"/></w:rPr><w:t xml:space="preserve">The tenant pays </w:t></w:r><w:del w:id="1" w:author="Alice" w:date="2026-01-02T00:00:00Z"><w:r><w:delText>500</w:delText></w:r></w:del><w:ins w:id="2" w:author="Alice" w:date="2026-01-02T00:00:00Z"><w:r><w:t>600</w:t></w:r></w:ins><w:r><w:t xml:space="preserve"> euros </w:t></w:r><w:commentRangeStart w:id="0"/><w:r><w:t>per month</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r><w:bookmarkStart w:id="0" w:name="pay"/><w:r><w:t>.</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p><w:p><w:pPr><w:spacing w:after="0"/></w:pPr></w:p><w:p><w:r><w:t>Second clause stays.</w:t></w:r></w:p>"#;

#[test]
fn noop_is_byte_identical() {
    let mut d = docx(WORDY);
    let before = d.dom.to_xml();
    let md = d.render(View::Review);
    let r = d.apply(&md, &opts(true)).unwrap();
    assert!(!r.changed(), "{r:?}");
    assert!(!d.is_dirty());
    assert_eq!(d.dom.to_xml(), before);
}

#[test]
fn renders_others_changes() {
    let d = docx(WORDY);
    let md = d.render(View::Review);
    assert!(md.contains(r#"<span style="color:#FF0000">The tenant pays</span> {--500--}{++600++} euros per month."#), "{md}");
    assert_eq!(uncolored(&d.render(View::Accepted)).lines().nth(2), Some("The tenant pays 600 euros per month."));
    assert_eq!(uncolored(&d.render(View::Original)).lines().nth(2), Some("The tenant pays 500 euros per month."));
}

#[test]
fn tracked_edit_keeps_formatting_and_others_changes() {
    let mut d = docx(WORDY);
    let md = d.render(View::Review).replace("tenant pays", "tenant always pays").replace("euros per month", "euros per calendar month").replace("Second clause stays.", "Second clause stays. Added.");
    let r = d.apply(&md, &opts(true)).unwrap();
    assert!(r.changed());
    let xml = d.dom.to_xml();
    // Alice's changes are untouched, our insertion is attributed to us.
    assert!(xml.contains(r#"<w:del w:id="1" w:author="Alice""#));
    assert!(xml.contains(r#"w:author="Me""#));
    // The inserted word inherits the neighbouring run's direct formatting.
    let ins = &xml[xml.find(r#"w:author="Me""#).unwrap()..];
    assert!(ins[..400].contains(r#"<w:color w:val="FF0000"/>"#), "{}", &ins[..400]);
    // Bookmark, comment anchors and the empty spacing paragraph survive.
    assert!(xml.contains(r#"<w:bookmarkStart w:id="0" w:name="pay"/>"#));
    assert!(xml.contains(r#"<w:commentRangeEnd w:id="0"/>"#));
    assert!(xml.contains(r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr></w:p>"#));
    let mut d2 = reopen(&mut d);
    let md2 = d2.render(View::Review);
    assert!(md2.contains("euros per {++calendar ++}month"), "{md2}");
    assert!(md2.contains("Second clause stays.{++ Added.++}"), "{md2}");
    // Accepting everything yields the clean text.
    d2.resolve_changes(&[], None, true).unwrap();
    assert!(d2.changes().is_empty());
    assert!(uncolored(&d2.render(View::Review)).contains("The tenant always pays 600 euros per calendar month."));
}

#[test]
fn untracked_edit_is_clean() {
    let mut d = docx(WORDY);
    let md = d.render(View::Review).replace("Second clause stays.", "Second clause changed.");
    d.apply(&md, &opts(false)).unwrap();
    let xml = d.dom.to_xml();
    assert!(!xml.contains(r#"w:author="Me""#));
    assert!(d.render(View::Review).contains("Second clause changed."));
    // Alice's changes remain pending.
    assert_eq!(d.changes().len(), 2);
}

#[test]
fn deleting_own_insertion_removes_it() {
    let mut d = docx(WORDY);
    let md = d.render(View::Review).replace("Second clause stays.", "Second clause stays. Oops.");
    d.apply(&md, &opts(true)).unwrap();
    let mut d = reopen(&mut d);
    let md = d.render(View::Review).replace("{++ Oops.++}", "");
    d.apply(&md, &opts(true)).unwrap();
    assert!(!d.dom.to_xml().contains("Oops"));
}

#[test]
fn delete_and_insert_paragraphs_then_reject_restores() {
    let mut d = docx(WORDY);
    let original = d.render(View::Accepted);
    let md = d.render(View::Review).replace("Second clause stays.", "- A new bullet\n\n## New heading");
    d.apply(&md, &opts(true)).unwrap();
    let mut d = reopen(&mut d);
    let md = d.render(View::Review);
    assert!(md.contains("{--Second clause stays.--}"), "{md}");
    assert!(md.contains("- {++A new bullet++}"), "{md}");
    assert!(md.contains("## {++New heading++}"), "{md}");
    let mine: Vec<usize> = d.changes().iter().filter(|c| c.author == "Me").map(|c| c.n).collect();
    d.resolve_changes(&mine, None, false).unwrap();
    assert_eq!(d.render(View::Accepted), original);
}

#[test]
fn heading_change_is_tracked_format() {
    let mut d = docx(WORDY);
    let md = d.render(View::Review).replace("# Contract", "## Contract");
    let r = d.apply(&md, &opts(true)).unwrap();
    assert_eq!(r.stats.kind_changes, 1);
    let xml = d.dom.to_xml();
    assert!(xml.contains(r#"<w:pStyle w:val="Heading2"/>"#));
    assert!(xml.contains("w:pPrChange"));
    let mut d = reopen(&mut d);
    d.resolve_changes(&[], Some("Me"), false).unwrap();
    assert!(d.render(View::Review).contains("# Contract\n"));
    assert!(d.dom.to_xml().contains(r#"<w:pStyle w:val="Heading1"/>"#));
}

#[test]
fn bold_toggle_is_format_change() {
    let mut d = docx(WORDY);
    let md = d.render(View::Review).replace("Second clause stays.", "Second **clause** stays.");
    let r = d.apply(&md, &opts(true)).unwrap();
    assert!(r.stats.formats > 0 && r.stats.insertions == 0, "{r:?}");
    let mut d = reopen(&mut d);
    assert!(d.render(View::Review).contains("Second **clause** stays."));
    assert_eq!(d.changes()[d.changes().len() - 1].kind, docex_core::revisions::ChangeKind::Format);
}

#[test]
fn comments_from_markdown_and_cli() {
    let mut d = docx(WORDY);
    let md = d.render(View::Review).replace("Second clause", "{==Second clause==}{>>Why?<<}");
    let r = d.apply(&md, &opts(true)).unwrap();
    assert_eq!(r.comments_added, 1);
    let mut d = reopen(&mut d);
    assert!(d.render(View::Review).contains("{==Second clause==}{>>Me: Why?<<}"));
    let id = d.add_comment("tenant", "Who exactly?", "Me", "2026-10-29T00:00:00Z").unwrap();
    d.reply_comment(&id, "The lessee.", "Bob", "2026-10-29T00:00:00Z").unwrap();
    d.set_comment_done(&id, true).unwrap();
    let mut d = reopen(&mut d);
    let list = d.comment_list();
    let c = list.iter().find(|c| c.id == id).unwrap();
    assert_eq!((c.anchor.as_str(), c.done), ("tenant", true));
    assert!(list.iter().any(|c| c.parent.as_deref() == Some(id.as_str()) && c.author == "Bob"));
    // Re-applying the rendered text must not duplicate comments.
    let md = d.render(View::Review);
    let r = d.apply(&md, &opts(true)).unwrap();
    assert!(!r.changed(), "{r:?}");
}

#[test]
fn track_setting_roundtrip() {
    let mut d = docx(WORDY);
    assert!(!d.track_changes().unwrap());
    d.set_track_changes(true).unwrap();
    let mut d = reopen(&mut d);
    assert!(d.track_changes().unwrap());
    d.set_track_changes(false).unwrap();
    let d = reopen(&mut d);
    assert!(!d.track_changes().unwrap());
}

#[test]
fn tricky_text_roundtrips() {
    let tricky = [
        "1. not a list", "# not a heading", "- not a bullet", "a * b _c_ `d` [e] &lt;f&gt; ~g~ \\h",
        "{++ not critic ++}", "AT&amp;T &amp;copy;", "> not a quote", "x | y", "  leading spaces", "trailing   ",
        "under_score_word _start", "++} --} a~>b", "+ plus", "=== setext?",
    ];
    let body: String = tricky
        .iter()
        .map(|t| format!(r#"<w:p><w:r><w:t xml:space="preserve">{t}</w:t></w:r></w:p>"#))
        .collect();
    let mut d = docx(&body);
    let before = d.dom.to_xml();
    let md = d.render(View::Review);
    let r = d.apply(&md, &opts(true)).unwrap();
    assert!(!r.changed(), "{md}\n{r:?}");
    assert_eq!(d.dom.to_xml(), before);
    // Each line reads back as its own literal text.
    let blocks = docex_core::mdparse::parse(&md);
    let texts: Vec<String> = blocks.iter().map(|b| b.text()).collect();
    for t in tricky {
        let t = docex_core::xml::unescape(t);
        assert!(texts.contains(&t.trim().to_string()), "{t:?} not in {texts:?}");
    }
}

#[test]
fn new_document_from_markdown() {
    let md = "# Title\n\nSome *text* with a [link](https://x.org).\n\n- a\n- b\n  - c\n\n1. one\n2. two\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";
    let (mut d, r) = Docx::new_from_markdown(md, None, "Me", std::path::Path::new(".")).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let d = reopen(&mut d);
    assert!(d.changes().is_empty());
    let out = d.render(View::Review);
    assert!(out.contains("# Title"));
    assert!(out.contains("Some *text* with a [link](https://x.org)."));
    assert!(out.contains("- a\n- b\n  - c"), "{out}");
    assert!(out.contains("1. one\n1. two"), "{out}");
    assert!(out.contains("| **1** | **2** |") || out.contains("| 1 | 2 |"), "{out}");
}

fn fixture(name: &str) -> Docx {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(name);
    Docx::open(&p).unwrap()
}

#[test]
fn edit_existing_footnote_tracked() {
    let mut d = fixture("rich.docx");
    let md = d.render(View::Review).replace("[^13]: The footnote text.", "[^13]: The revised footnote text.\n\n    A second paragraph.");
    let r = d.apply(&md, &opts(true)).unwrap();
    assert!(r.changed(), "{r:?}");
    let mut d = reopen(&mut d);
    let md = d.render(View::Review);
    assert!(md.contains("[^13]: The {++revised ++}footnote text.\n\n    {++A second paragraph.++}"), "{md}");
    let ch = d.changes();
    assert!(ch.iter().all(|c| c.part == "footnotes"), "{ch:?}");
    d.resolve_changes(&[], None, true).unwrap();
    let d = reopen(&mut d);
    assert!(d.changes().is_empty());
    assert!(d.render(View::Review).contains("[^13]: The revised footnote text.\n\n    A second paragraph."));
}

#[test]
fn new_footnote_creates_part_and_reference() {
    let mut d = docx(WORDY);
    let md = d.render(View::Review).replace("Second clause stays.", "Second clause stays.[^n1]\n\n[^n1]: Says who?");
    let r = d.apply(&md, &opts(true)).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let d = reopen(&mut d);
    let md = d.render(View::Review);
    assert!(md.contains("Second clause stays.{++[^1]++}"), "{md}");
    assert!(md.contains("[^1]: {++Says who?++}"), "{md}");
    let xml = d.dom.to_xml();
    assert!(xml.contains(r#"<w:footnoteReference w:id="1"/>"#));
    assert!(d.pkg.has("word/footnotes.xml"));
    assert!(d.pkg.read_string("[Content_Types].xml").unwrap().contains("footnotes+xml"));
}

const BOXED: &str = r#"<w:p><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wp:anchor><wp:docPr id="1" name="Text Box 1"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:txbx><w:txbxContent><w:p><w:r><w:t>Box text here.</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict><v:shape><v:textbox><w:txbxContent><w:p><w:r><w:t>Box text here.</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r><w:r><w:t>Anchor text.</w:t></w:r></w:p><w:p><w:r><w:t>After.</w:t></w:r></w:p>"#;

#[test]
fn text_box_edit_updates_both_copies() {
    let mut d = docx(BOXED);
    let md = d.render(View::Review);
    assert!(md.contains("<!-- docex:textbox tb1 -->\n\nBox text here.\n\n<!-- /docex:textbox -->"), "{md}");
    let before = d.dom.to_xml();
    assert!(!d.apply(&md, &opts(true)).unwrap().changed());
    assert_eq!(d.dom.to_xml(), before);
    let md = md.replace("Box text here.", "Box text changed here.\n\nAnd more.");
    d.apply(&md, &opts(true)).unwrap();
    let mut d = reopen(&mut d);
    let xml = d.dom.to_xml();
    assert_eq!(xml.matches("changed ").count(), 2, "both copies patched: {xml}");
    assert!(d.render(View::Review).contains("Box text {++changed ++}here.\n\n{++And more.++}"));
    // Changes are listed once although stored twice; accepting updates both copies.
    let n = d.changes().len();
    d.resolve_changes(&[], None, true).unwrap();
    let d = reopen(&mut d);
    assert!(d.changes().is_empty(), "had {n}");
    let xml = d.dom.to_xml();
    assert_eq!(xml.matches("changed ").count(), 2);
    assert!(!xml.contains("<w:ins "), "{xml}");
    assert!(d.render(View::Review).contains("\n\nBox text changed here.\n\nAnd more.\n\n"));
}

#[test]
fn captions() {
    let body = r#"<w:p><w:pPr><w:pStyle w:val="Caption"/></w:pPr><w:r><w:t xml:space="preserve">Figure </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> SEQ Figure \* ARABIC </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t>: A cat.</w:t></w:r></w:p><w:p><w:r><w:t>: not a caption</w:t></w:r></w:p>"#;
    let mut d = docx(body);
    let md = d.render(View::Review);
    assert!(md.contains("\n: Figure 1: A cat.\n"), "{md}");
    assert!(md.contains("\\: not a caption"), "{md}");
    assert!(!d.apply(&md, &opts(true)).unwrap().changed());
    let md = md.replace(": Figure 1: A cat.", ": Figure 1: A black cat.\n\n: Figure 2: A dog.");
    d.apply(&md, &opts(false)).unwrap();
    let d = reopen(&mut d);
    let xml = d.dom.to_xml();
    assert_eq!(xml.matches("SEQ Figure").count(), 2, "{xml}");
    assert_eq!(xml.matches(r#"<w:pStyle w:val="Caption"/>"#).count(), 2);
    assert!(d.render(View::Review).contains(": Figure 2: A dog."));
}

#[test]
fn model_roundtrip_is_noop_and_edits_track() {
    use docex_core::docmodel::{DocModel, MBlock};
    for mut d in [docx(WORDY), docx(BOXED), fixture("rich.docx")] {
        let before = d.dom.to_xml();
        let m = d.model();
        // Through JSON, as the GUI does.
        let m: DocModel = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        let r = d.apply_model(&m, &opts(true)).unwrap();
        assert!(!r.changed(), "{r:?}");
        assert_eq!(d.dom.to_xml(), before);
    }
    let mut d = docx(WORDY);
    let mut m = d.model();
    // Alice's change carries author and Word id.
    let MBlock::Para(p) = &m.blocks[1] else { panic!() };
    let ins = p.spans.iter().find(|s| s.t == "600").unwrap();
    assert_eq!((ins.author.as_deref(), ins.wid.as_deref()), (Some("Alice"), Some("2")));
    // Edit: append text as a typed (tracked) insertion, delete "Second ".
    let MBlock::Para(p) = &mut m.blocks[2] else { panic!() };
    p.spans[0].t = "clause stays.".into();
    p.spans.push(docex_core::docmodel::Span { t: " New.".into(), st: docex_core::model::Status::Ins, ..Default::default() });
    let r = d.apply_model(&m, &opts(true)).unwrap();
    assert!(r.changed());
    let md = d.render(View::Review);
    assert!(md.contains("{--Second --}clause stays.{++ New.++}"), "{md}");
    // Accept Alice's insertion by its Word id.
    assert_eq!(d.resolve_wids(&["2".into()], true).unwrap(), 1);
    assert!(uncolored(&d.render(View::Review)).contains("pays {--500--}600 euros"));
}

#[test]
fn model_noop_on_rich_fixture_spans() {
    let d = fixture("rich.docx");
    let m = d.model();
    let j = serde_json::to_string(&m).unwrap();
    assert!(j.contains(r#""obj":"img:media/rId10.png""#), "{j}");
    assert!(j.contains(r#""obj":"fn:13""#));
    assert!(j.contains(r#""label":"13""#));
}

#[test]
fn markdown_paste_model() {
    use docex_core::docmodel::{from_markdown, MBlock};
    let m = from_markdown("# Title\n\n- one **two**\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
    let kinds: Vec<String> = m
        .blocks
        .iter()
        .map(|b| match b {
            MBlock::Para(p) => p.kind.clone(),
            MBlock::Table { rows, .. } => format!("table{}", rows.len()),
            MBlock::Control { .. } => "control".into(),
            MBlock::Opaque { .. } => "opaque".into(),
        })
        .collect();
    assert_eq!(kinds, ["h1", "bullet", "table2"]);
    let MBlock::Para(p) = &m.blocks[1] else { panic!() };
    assert!(p.spans.iter().any(|s| s.t == "two" && s.b));
}

#[test]
fn equations_roundtrip_and_track() {
    // Equations written by pandoc (independent OMML) read back as LaTeX.
    let mut d = fixture("pandoc-math.docx");
    let before = d.dom.to_xml();
    let md = d.render(View::Review);
    assert!(md.contains("$\\sqrt{a^2+b^2}$"), "{md}");
    assert!(md.contains("$\\sum_{k=0}^\\infty \\frac{x^k}{k!}$"), "{md}");
    assert!(md.contains("\\begin{pmatrix} 1 & 0 \\\\ 0 & 1 \\end{pmatrix}"), "{md}");
    assert!(!d.apply(&md, &opts(true)).unwrap().changed());
    assert_eq!(d.dom.to_xml(), before);
    // Editing a formula is a tracked replacement of the equation.
    let mut d = fixture("math.docx");
    let md = d.render(View::Review).replace("$E=mc^2$", "$E=mc^3$");
    d.apply(&md, &opts(true)).unwrap();
    let d = reopen(&mut d);
    let md = d.render(View::Review);
    assert!(md.contains("{--$E=mc^2$--}{++$E=mc^3$++}"), "{md}");
    assert!(d.dom.to_xml().contains("<w:ins ") && d.dom.to_xml().contains("<m:oMath>"));
    // A new display equation becomes m:oMathPara.
    let mut d = docx(WORDY);
    let md = d.render(View::Review).replace("Second clause stays.", "Second clause stays.\n\n$$\\frac{a}{b}$$");
    d.apply(&md, &opts(false)).unwrap();
    let xml = d.dom.to_xml();
    assert!(xml.contains("<m:oMathPara><m:oMath><m:f>"), "{xml}");
    assert!(d.render(View::Review).contains("$$\\frac{a}{b}$$"));
}

#[test]
fn review_ids_are_stable_and_resolve_together() {
    let mut d = docx(WORDY);
    // Two more changes by Me: an insertion and a replacement.
    let md = d.render(View::Review).replace("Second clause stays.", "Second clause remains. Added.");
    d.apply(&md, &opts(true)).unwrap();
    let mut d = reopen(&mut d);
    let rv = d.review();
    let alice: Vec<&docex_core::review::Item> = rv.items.iter().filter(|i| i.author == "Alice").collect();
    // Alice's "500" -> "600" reads as one replacement.
    assert_eq!(alice.len(), 1, "{:?}", rv.items);
    assert_eq!((alice[0].kind.as_str(), alice[0].old.as_deref(), alice[0].new.as_deref()), ("replace", Some("500"), Some("600")));
    assert_eq!(alice[0].section, vec!["Contract".to_string()]);
    assert!(alice[0].before.ends_with("pays "), "{:?}", alice[0]);
    let mine: Vec<String> = rv.items.iter().filter(|i| i.author == "Me").map(|i| i.id.clone()).collect();
    assert!(!mine.is_empty());
    // Accept Alice, reject mine, in one call; order of decisions doesn't matter.
    let (a, r) = d.resolve_ids(&[alice[0].id.clone()], &mine).unwrap();
    assert!(a >= 1 && r >= 1);
    assert!(d.changes().is_empty());
    let md = uncolored(&d.render(View::Review));
    assert!(md.contains("pays 600 euros") && md.contains("Second clause stays."), "{md}");
    // Unknown ids are refused before anything changes.
    assert!(d.resolve_ids(&["c999".into()], &[]).is_err());
}

#[test]
fn ids_view_reapplies_as_noop_and_comments_need_unique_quotes() {
    let mut d = docx(WORDY);
    let before = d.dom.to_xml();
    let md = d.render_with_ids();
    assert!(md.contains("{--500--}⟦c1⟧{++600++}⟦c2⟧"), "{md}");
    assert!(!d.apply(&md, &opts(true)).unwrap().changed());
    assert_eq!(d.dom.to_xml(), before);
    // "e" occurs many times: refuse, then place exactly with nth / section.
    let err = d.add_comment_at("e", None, None, "x", "Me", "2026-01-01T00:00:00Z").unwrap_err().to_string();
    assert!(err.contains("occurs") && err.contains("--nth"), "{err}");
    let id = d.add_comment_at("clause", None, Some("Contract"), "why?", "Me", "2026-01-01T00:00:00Z").unwrap();
    let d = reopen(&mut d);
    let c = d.comment_list().into_iter().find(|c| c.id == id).unwrap();
    assert_eq!(c.anchor, "clause");
}

// ---------------------------------------------------------------------------
// Regressions from the code review

const D: &str = r#"w:author="Alice" w:date="2026-01-01T00:00:00Z""#;

#[test]
fn reject_style_and_mark_format_together_does_not_panic() {
    let body = format!(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/><w:rPr><w:b/><w:rPrChange w:id="1" {D}><w:rPr/></w:rPrChange></w:rPr><w:pPrChange w:id="2" {D}><w:pPr><w:pStyle w:val="Normal"/><w:ind w:left="720"/></w:pPr></w:pPrChange></w:pPr><w:r><w:t>Text</w:t></w:r></w:p>"#
    );
    let mut d = docx(&body);
    d.resolve_changes(&[], None, false).unwrap();
    let xml = d.dom.to_xml();
    assert!(xml.contains(r#"<w:pStyle w:val="Normal"/><w:ind w:left="720"/>"#), "{xml}");
    assert!(!xml.contains("Change") && !xml.contains("<w:b/>"), "{xml}");
    let mut d = docx(&body);
    d.resolve_ids(&[], &["c1".into(), "c2".into()]).unwrap();
    assert!(d.changes().is_empty());
}

#[test]
fn reject_mark_format_keeps_mark_insertion() {
    let body = format!(
        r#"<w:p><w:pPr><w:rPr><w:ins w:id="1" {D}/><w:b/><w:rPrChange w:id="2" w:author="Bob" w:date="2026-01-01T00:00:00Z"><w:rPr/></w:rPrChange></w:rPr></w:pPr><w:r><w:t>Text</w:t></w:r></w:p>"#
    );
    let mut d = docx(&body);
    d.resolve_ids(&[], &["c2".into()]).unwrap();
    let ch = d.changes();
    assert_eq!(ch.len(), 1, "{ch:?}");
    assert_eq!(ch[0].author, "Alice");
}

#[test]
fn tracked_changes_inside_equations() {
    let body = format!(
        r#"<w:p><w:r><w:t xml:space="preserve">Formula </w:t></w:r><m:oMath><m:r><w:rPr><w:del w:id="7" {D}/></w:rPr><m:t>x+1</m:t></m:r><m:r><w:rPr><w:ins w:id="8" {D}/></w:rPr><m:t>y+2</m:t></m:r></m:oMath><w:r><w:t xml:space="preserve"> end.</w:t></w:r></w:p>"#
    );
    let d = docx(&body);
    let kinds: Vec<_> = d.changes().iter().map(|c| (c.kind, c.text.clone())).collect();
    use docex_core::revisions::ChangeKind::*;
    assert_eq!(kinds, vec![(Delete, "x+1".to_string()), (Insert, "y+2".to_string())]);
    // Not shown as one formula mixing old and new.
    assert!(d.render(View::Review).contains("Formula <!--docex:math--> end."), "{}", d.render(View::Review));
    let mut a = docx(&body);
    a.resolve_changes(&[], None, true).unwrap();
    let xml = a.dom.to_xml();
    assert!(!xml.contains("x+1") && xml.contains("y+2") && !xml.contains("w:ins"), "{xml}");
    assert!(a.render(View::Review).contains("$y+2$"));
    let mut r = docx(&body);
    r.resolve_changes(&[], None, false).unwrap();
    let xml = r.dom.to_xml();
    assert!(xml.contains("x+1") && !xml.contains("y+2") && !xml.contains("w:del"), "{xml}");
}

#[test]
fn accepting_deletion_of_whole_table_removes_it() {
    let mut d = docx(r#"<w:p><w:r><w:t>Before</w:t></w:r></w:p><w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>After</w:t></w:r></w:p>"#);
    let md = d.render(View::Review).replace("| cell |\n| --- |\n\n", "").replace("| cell |\n| --- |", "");
    d.apply(&md, &opts(true)).unwrap();
    let mut d = reopen(&mut d);
    assert!(d.dom.to_xml().contains("<w:tbl>"));
    d.resolve_changes(&[], None, true).unwrap();
    assert!(!d.dom.to_xml().contains("<w:tbl>"), "{}", d.dom.to_xml());
}

#[test]
fn critic_tokens_in_code_are_literal() {
    let mut d = docx(r#"<w:p><w:r><w:t xml:space="preserve">Use </w:t></w:r><w:r><w:rPr><w:rStyle w:val="VerbatimChar"/></w:rPr><w:t>if(x){--i;}</w:t></w:r><w:r><w:t xml:space="preserve"> to decrement.</w:t></w:r></w:p><w:p><w:r><w:t>Second paragraph here.</w:t></w:r></w:p>"#);
    let md = d.render(View::Review);
    assert!(md.contains("`if(x){--i;}`"), "{md}");
    let r = d.apply(&md.replace("paragraph here", "paragraph NEW here"), &opts(true)).unwrap();
    assert!(r.changed(), "{r:?}");
    assert!(d.render(View::Review).contains("Second paragraph {++NEW ++}here."));
    // An unclosed marker doesn't swallow later paragraphs.
    let mut d = docx(WORDY);
    let md = d.render(View::Review).replace("Second clause stays.", "Second {--clause stays.\n\nThird para.");
    d.apply(&md, &opts(true)).unwrap();
    assert!(d.render(View::Review).contains("Third para."), "{}", d.render(View::Review));
}

#[test]
fn break_then_indented_marker_and_placeholder_in_box() {
    let mut d = docx(r#"<w:p><w:r><w:t>Items:</w:t><w:br/><w:t xml:space="preserve"> - first</w:t></w:r></w:p><w:p><w:r><w:t>Next.</w:t></w:r></w:p>"#);
    let md = d.render(View::Review);
    assert!(!d.apply(&md, &opts(true)).unwrap().changed(), "{md}");
    let boxed = BOXED.replace("<w:r><w:t>Box text here.</w:t></w:r>", r#"<w:r><w:sym w:font="Symbol" w:char="F0B7"/></w:r><w:r><w:t>Box text here.</w:t></w:r>"#);
    let mut d = docx(&boxed);
    let md = d.render(View::Review);
    assert!(!d.apply(&md, &opts(true)).unwrap().changed(), "{md}");
}

#[test]
fn point_comment_at_paragraph_start() {
    let mut d = docx(WORDY);
    let md = d.render(View::Review).replace("Second clause stays.", "{>>note<<}Second clause stays.");
    let r = d.apply(&md, &opts(true)).unwrap();
    assert_eq!(r.comments_added, 1);
    let d = reopen(&mut d);
    assert!(d.comment_list().iter().any(|c| c.text == "note" && c.anchor == "S"));
}

#[test]
fn unrelated_accept_leaves_text_box_fallback_alone() {
    let boxed = BOXED.replacen("<w:t>Box text here.</w:t>", "<w:t>Box text here.</w:t>", 1).replace(
        "<v:textbox><w:txbxContent><w:p><w:r><w:t>Box text here.</w:t>",
        "<v:textbox><w:txbxContent><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Box text here.</w:t>",
    );
    let body = format!(r#"{boxed}<w:p><w:ins w:id="9" {D}><w:r><w:t>Added.</w:t></w:r></w:ins></w:p>"#);
    let mut d = docx(&body);
    d.resolve_changes(&[], None, true).unwrap();
    assert!(d.dom.to_xml().contains("<w:rPr><w:b/></w:rPr><w:t>Box text here.</w:t>"));
}

#[test]
fn deleting_anchor_paragraph_keeps_box_text_plain() {
    let mut d = docx(BOXED);
    let md = d.render(View::Review).replace("<!--docex:shape-->Anchor text.", "");
    d.apply(&md, &opts(true)).unwrap();
    let xml = d.dom.to_xml();
    let inside: Vec<&str> = xml.split("<w:txbxContent>").skip(1).map(|s| s.split("</w:txbxContent>").next().unwrap()).collect();
    assert!(inside.iter().all(|b| b.contains("<w:t>Box text here.</w:t>") && !b.contains("delText")), "{xml}");
}

#[test]
fn change_ids_are_unique_across_wrappers() {
    let mut d = docx(r#"<w:p><w:r><w:t xml:space="preserve">Visit </w:t></w:r><w:hyperlink w:anchor="x"><w:r><w:t>the site</w:t></w:r></w:hyperlink><w:r><w:t xml:space="preserve"> today please.</w:t></w:r></w:p>"#);
    let md = d.render(View::Review).replace("[the site](#x) today ", "");
    d.apply(&md, &opts(true)).unwrap();
    let xml = d.dom.to_xml();
    let ids: Vec<&str> = xml.split("<w:del w:id=\"").skip(1).map(|s| s.split('"').next().unwrap()).collect();
    assert!(ids.len() >= 2, "{xml}");
    let uniq: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(uniq.len(), ids.len(), "{ids:?}");
}

#[test]
fn keep_opaque_preserves_unmatched_placeholders() {
    let body = r#"<w:sdt><w:sdtPr><w:docPartObj><w:docPartGallery w:val="Table of Contents"/></w:docPartObj></w:sdtPr><w:sdtContent><w:p><w:r><w:t>TOC</w:t></w:r></w:p></w:sdtContent></w:sdt><w:p><w:r><w:t>Body text.</w:t></w:r></w:p>"#;
    let mut d = docx(body);
    let mut o = opts(false);
    o.keep_opaque = true;
    d.apply("Body text changed.\n", &o).unwrap();
    assert!(d.dom.to_xml().contains("Table of Contents"));
    assert!(d.render(View::Review).contains("Body text changed."));
}

// A table as Word writes it: a header cell over two columns, a shaded cell over two rows.
const MERGED: &str = r#"<w:p><w:r><w:t>Before</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="6000" w:type="dxa"/><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>Wide</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:vMerge w:val="restart"/><w:shd w:val="clear" w:fill="FFFF00"/></w:tcPr><w:p><w:r><w:t>Tall</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:vMerge/><w:shd w:val="clear" w:fill="FFFF00"/></w:tcPr><w:p/></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>c</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>d</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>e</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>After</w:t></w:r></w:p>"#;

const MERGED_MD: &str = "<table>\n<tr>\n<td colspan=\"2\">Wide</td>\n<td rowspan=\"2\">Tall</td>\n</tr>\n<tr>\n<td>a</td>\n<td>b</td>\n</tr>\n<tr>\n<td>c</td>\n<td>d</td>\n<td>e</td>\n</tr>\n</table>";

/// Apply an edit of the merged table; return the reopened document, its
/// rendering and the warnings.
fn edit_merged(tracked: bool, f: impl Fn(&str) -> String) -> (Docx, String, Vec<String>) {
    let mut d = docx(MERGED);
    let md = d.render(View::Review);
    let new = f(&md);
    assert_ne!(new, md, "the edit didn't apply");
    let r = d.apply(&new, &opts(tracked)).unwrap();
    let mut d = reopen(&mut d);
    let out = d.render(View::Review);
    // What docex writes reads back the same.
    let again = d.apply(&out, &opts(tracked)).unwrap();
    assert!(!again.changed(), "not stable: {out}\n{again:?}");
    (d, out, r.warnings)
}

#[test]
fn merged_cells_render_as_html_and_noop_is_identical() {
    let mut d = docx(MERGED);
    let md = d.render(View::Review);
    assert!(md.contains(MERGED_MD), "{md}");
    let before = d.dom.to_xml();
    let r = d.apply(&md, &opts(true)).unwrap();
    assert!(!r.changed(), "{r:?}");
    assert_eq!(d.dom.to_xml(), before);
    // The GUI model round-trips too.
    let m = d.model();
    let r = d.apply_model(&m, &opts(true)).unwrap();
    assert!(!r.changed(), "{r:?}");
    assert_eq!(d.dom.to_xml(), before);
}

#[test]
fn merged_cell_text_edit_is_tracked_and_keeps_layout() {
    let (d, out, w) = edit_merged(true, |md| md.replace(">Tall<", ">Taller<").replace(">a<", ">alpha<"));
    assert!(w.is_empty(), "{w:?}");
    assert!(out.contains("<td rowspan=\"2\">{--Tall--}{++Taller++}</td>"), "{out}");
    assert!(out.contains("<td>{--a--}{++alpha++}</td>"), "{out}");
    let xml = d.dom.to_xml();
    assert!(xml.contains(r#"<w:gridSpan w:val="2"/>"#) && xml.contains(r#"<w:vMerge w:val="restart"/>"#) && xml.contains(r#"w:fill="FFFF00""#), "{xml}");
}

#[test]
fn markup_inside_html_cells() {
    let (_, out, _) = edit_merged(true, |md| md.replace(">b<", ">**bold** and {++new++} $x^2$<").replace(">c<", ">{==c==}{>>check<<}<"));
    assert!(out.contains("<td>{--b--}{++**bold** and ++}{++new++}{++ $x^2$++}</td>") || out.contains("**bold**"), "{out}");
    assert!(out.contains("$x^2$") && out.contains("{>>Me: check<<}"), "{out}");
}

#[test]
fn row_added_to_merged_table_is_tracked() {
    let (d, out, _) = edit_merged(true, |md| md.replace("<td>e</td>\n</tr>\n", "<td>e</td>\n</tr>\n<tr>\n<td>f</td>\n<td>g</td>\n<td>h</td>\n</tr>\n"));
    assert!(out.contains("<td>{++f++}</td>"), "{out}");
    let xml = d.dom.to_xml();
    assert_eq!(xml.matches("<w:tr>").count() + xml.matches("<w:tr ").count(), 4, "{xml}");
    assert!(xml.contains("<w:trPr><w:ins "), "{xml}");
}

#[test]
fn merge_split_and_extend_cells() {
    // Merge c and d.
    let (d, out, w) = edit_merged(true, |md| md.replace("<td>c</td>\n<td>d</td>", "<td colspan=\"2\">c d</td>"));
    assert!(out.contains("<td colspan=\"2\">c{++ d++}</td>") || out.contains("<td colspan=\"2\">{++c ++}d</td>"), "{out}");
    assert!(w.iter().any(|w| w.contains("not recorded as tracked")), "{w:?}");
    assert_eq!(d.dom.to_xml().matches(r#"<w:gridSpan w:val="2"/>"#).count(), 2);
    // Split the wide cell.
    let (_, out, _) = edit_merged(false, |md| md.replace("<td colspan=\"2\">Wide</td>", "<td>Wide</td>\n<td>New</td>"));
    assert!(out.contains("<td>Wide</td>\n<td>New</td>\n<td rowspan=\"2\">Tall</td>"), "{out}");
    // Let the tall cell reach the last row.
    let (d, out, _) = edit_merged(false, |md| md.replace("rowspan=\"2\"", "rowspan=\"3\"").replace("<td>e</td>\n", ""));
    assert!(out.contains("<td rowspan=\"3\">Tall</td>") && !out.contains(">e<"), "{out}");
    let xml = d.dom.to_xml();
    assert_eq!(xml.matches("<w:vMerge/>").count(), 2, "{xml}");
    assert!(xml.matches(r#"w:fill="FFFF00""#).count() == 3, "shading follows the merge: {xml}");
}

#[test]
fn columns_added_and_removed() {
    let (d, out, _) = edit_merged(false, |md| {
        md.replace("<td rowspan=\"2\">Tall</td>", "<td rowspan=\"2\">Tall</td>\n<td>X1</td>").replace("<td>b</td>", "<td>b</td>\n<td>X2</td>").replace("<td>e</td>", "<td>e</td>\n<td>X3</td>")
    });
    assert!(out.contains("<td rowspan=\"2\">Tall</td>\n<td>X1</td>"), "{out}");
    let xml = d.dom.to_xml();
    assert_eq!(xml.matches("<w:gridCol ").count(), 4, "{xml}");
    // Widths keep the table's total.
    let total: i64 = xml.split("<w:gridCol w:w=\"").skip(1).map(|s| s.split('"').next().unwrap().parse::<i64>().unwrap()).sum();
    assert!((8990..=9000).contains(&total), "{total}");
    // A simple table gets a column too (once it couldn't).
    let mut s = docx(r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>h1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>h2</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>2</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
    let md = s.render(View::Review).replace("| h1 | h2 |\n| --- | --- |\n| 1 | 2 |", "| h1 | h2 | h3 |\n| --- | --- | --- |\n| 1 | 2 | 3 |");
    let r = s.apply(&md, &opts(false)).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert!(reopen(&mut s).render(View::Review).contains("| h1 | h2 | h3 |\n| --- | --- | --- |\n| 1 | 2 | 3 |"));
}

#[test]
fn new_table_with_merged_cells() {
    let mut d = docx(WORDY);
    let md = d.render(View::Review) + "\n" + MERGED_MD + "\n";
    d.apply(&md, &opts(false)).unwrap();
    let mut d = reopen(&mut d);
    let out = d.render(View::Review);
    assert!(out.contains("<td colspan=\"2\">**Wide**</td>\n<td rowspan=\"2\">**Tall**</td>"), "{out}");
    assert!(out.contains("<td>c</td>\n<td>d</td>\n<td>e</td>"), "{out}");
    assert!(!d.apply(&out, &opts(false)).unwrap().changed());
}

const NUMBERING: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="10"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:pStyle w:val="Heading1"/><w:lvlText w:val="%1"/></w:lvl><w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:pStyle w:val="Clause"/><w:lvlText w:val="%1.%2"/></w:lvl><w:lvl w:ilvl="2"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="(%3)"/></w:lvl></w:abstractNum><w:abstractNum w:abstractNumId="11"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="lowerRoman"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:abstractNum w:abstractNumId="12"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="Article %1"/></w:lvl><w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:isLgl/><w:lvlText w:val="%1.%2"/></w:lvl></w:abstractNum><w:num w:numId="10"><w:abstractNumId w:val="10"/></w:num><w:num w:numId="11"><w:abstractNumId w:val="11"/></w:num><w:num w:numId="12"><w:abstractNumId w:val="11"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="1"/></w:lvlOverride></w:num><w:num w:numId="13"><w:abstractNumId w:val="12"/></w:num></w:numbering>"#;

// "Clause" is numbered through its style, its level comes from the list's pStyle.
const CLAUSE_STYLE: &str = r#"<w:style w:type="paragraph" w:styleId="Clause"><w:name w:val="Clause"/><w:basedOn w:val="Normal"/><w:pPr><w:numPr><w:numId w:val="10"/></w:numPr></w:pPr></w:style>"#;

fn numbered(style: &str, num: Option<(u32, u32)>, text: &str) -> String {
    let st = if style.is_empty() { String::new() } else { format!(r#"<w:pStyle w:val="{style}"/>"#) };
    let np = num.map(|(n, l)| format!(r#"<w:numPr><w:ilvl w:val="{l}"/><w:numId w:val="{n}"/></w:numPr>"#)).unwrap_or_default();
    format!(r#"<w:p><w:pPr>{st}{np}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

#[test]
fn automatic_numbers_are_worked_out() {
    let body = [
        numbered("Heading1", Some((10, 0)), "Scope"),
        numbered("Clause", None, "First clause."),
        numbered("Clause", None, "Second clause."),
        numbered("", Some((10, 2)), "A point."),
        numbered("", Some((10, 2)), "Another point."),
        numbered("Heading1", Some((10, 0)), "Liability"),
        numbered("Clause", None, "Limited."),
        numbered("", Some((11, 0)), "one"),
        numbered("", Some((11, 0)), "two"),
        numbered("", None, "Plain text."),
        numbered("", Some((12, 0)), "restarted"),
        numbered("", Some((13, 0)), "Definitions"),
        numbered("", Some((13, 1)), "Legal sub-clause"),
    ]
    .concat();
    let mut d = docx_with(&body, Some(NUMBERING), CLAUSE_STYLE);
    let md = d.render_numbered(View::Review);
    for want in [
        "# 1 Scope",
        "1.1 First clause.",
        "1.2 Second clause.",
        "(a) A point.",
        "(b) Another point.",
        "# 2 Liability",
        "2.1 Limited.",
        "i. one",
        "ii. two",
        "i. restarted",
        "Article I Definitions",
        "1.1 Legal sub-clause",
    ] {
        assert!(md.contains(want), "missing {want:?} in\n{md}");
    }
    // Only a view: the normal Markdown has no numbers, and applying it changes nothing.
    let plain = d.render(View::Review);
    assert!(plain.contains("# Scope") && !plain.contains("1.1 First"), "{plain}");
    assert!(!d.apply(&plain, &opts(true)).unwrap().changed());
    // The GUI model carries them; review sections name them.
    let m = d.model();
    let labels: Vec<String> = m.blocks.iter().filter_map(|b| match b {
        docex_core::docmodel::MBlock::Para(p) => p.label.clone(),
        _ => None,
    }).collect();
    assert_eq!(labels[..3], ["1", "1.1", "1.2"]);
    assert!(!d.apply_model(&m, &opts(true)).unwrap().changed());
}

// Content controls as Word writes them: a field still showing its placeholder,
// a cover page (with empty spacing paragraphs), a locked one, a table of contents.
const CONTROLS: &str = r#"<w:sdt><w:sdtPr><w:id w:val="-1"/><w:docPartObj><w:docPartGallery w:val="Cover Pages"/><w:docPartUnique/></w:docPartObj></w:sdtPr><w:sdtContent><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Annual Report</w:t></w:r></w:p><w:p/><w:p><w:r><w:t>Prepared for the board.</w:t></w:r></w:p></w:sdtContent></w:sdt><w:p><w:r><w:t>Customer:</w:t></w:r></w:p><w:sdt><w:sdtPr><w:alias w:val="Customer"/><w:tag w:val="customer"/><w:id w:val="123"/><w:showingPlcHdr/></w:sdtPr><w:sdtEndPr/><w:sdtContent><w:p><w:r><w:rPr><w:rStyle w:val="PlaceholderText"/></w:rPr><w:t>Click here to enter text.</w:t></w:r></w:p></w:sdtContent></w:sdt><w:sdt><w:sdtPr><w:alias w:val="Legal notice"/><w:lock w:val="contentLocked"/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>Do not change this text.</w:t></w:r></w:p></w:sdtContent></w:sdt><w:sdt><w:sdtPr><w:docPartObj><w:docPartGallery w:val="Table of Contents"/></w:docPartObj></w:sdtPr><w:sdtContent><w:p><w:r><w:t>Contents</w:t></w:r></w:p></w:sdtContent></w:sdt><w:p><w:r><w:t>Body text.</w:t></w:r></w:p>"#;

#[test]
fn content_controls_show_their_paragraphs() {
    let mut d = docx(CONTROLS);
    let md = d.render(View::Review);
    for want in [
        "<!-- docex:control b0 Cover Pages -->\n\n# Annual Report\n\nPrepared for the board.\n\n<!-- /docex:control -->",
        "<!-- docex:control b2 Customer -->\n\nClick here to enter text.\n\n<!-- /docex:control -->",
        "<!-- docex:control b3 Legal notice -->",
        "<!-- docex:opaque b4 Table of Contents -->",
    ] {
        assert!(md.contains(want), "missing {want:?} in\n{md}");
    }
    let before = d.dom.to_xml();
    assert!(!d.apply(&md, &opts(true)).unwrap().changed());
    assert!(!d.apply_model(&d.model(), &opts(true)).unwrap().changed());
    assert_eq!(d.dom.to_xml(), before);
}

#[test]
fn editing_content_controls() {
    let mut d = docx(CONTROLS);
    let md = d
        .render(View::Review)
        .replace("Click here to enter text.", "ACME GmbH")
        .replace("Prepared for the board.", "Prepared for the supervisory board.")
        .replace("Do not change this text.", "Changed anyway.");
    let r = d.apply(&md, &opts(true)).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("Legal notice") && w.contains("locked")), "{:?}", r.warnings);
    let mut d = reopen(&mut d);
    let out = d.render(View::Review);
    assert!(out.contains("<!-- docex:control b2 Customer -->\n\n{++ACME GmbH++}\n\n"), "{out}");
    assert!(out.contains("Prepared for the {++supervisory ++}board."), "{out}");
    assert!(out.contains("Do not change this text."), "{out}");
    let xml = d.dom.to_xml();
    // The control stays, its placeholder flag goes; the cover page keeps its empty paragraph.
    assert!(xml.contains(r#"<w:alias w:val="Customer"/>"#) && !xml.contains("showingPlcHdr"), "{xml}");
    assert!(xml.contains("<w:p/>"), "{xml}");
    assert!(!d.apply(&out, &opts(true)).unwrap().changed());
    // Numbers and review sections see the paragraphs too.
    assert!(d.review().items.iter().any(|i| i.section.first().is_some_and(|s| s == "Annual Report")));
}

#[test]
fn underline_highlight_color_and_raised_text() {
    let run = |rpr: &str, t: &str| format!(r#"<w:r><w:rPr>{rpr}</w:rPr><w:t xml:space="preserve">{t}</w:t></w:r>"#);
    let body = format!(
        "<w:p>{}{}{}{}{}{}{}{}</w:p><w:p><w:r><w:t>Plain.</w:t></w:r></w:p>",
        run(r#"<w:u w:val="single"/>"#, "Defined Term"),
        run("", " means "),
        run(r#"<w:highlight w:val="yellow"/>"#, "check this"),
        run("", ", "),
        run(r#"<w:color w:val="C00000"/>"#, "open point"),
        run(r#"<w:color w:val="595959"/>"#, " grey is not a colour, 10"),
        run(r#"<w:vertAlign w:val="superscript"/>"#, "6"),
        run(r#"<w:highlight w:val="green"/><w:vertAlign w:val="subscript"/>"#, "2"),
    );
    let mut d = docx(&body);
    let md = d.render(View::Review);
    assert!(
        md.contains(r#"<u>Defined Term</u> means <mark>check this</mark>, <span style="color:#C00000">open point</span> grey is not a colour, 10<sup>6</sup><sub><mark data-color="green">2</mark></sub>"#),
        "{md}"
    );
    let before = d.dom.to_xml();
    assert!(!d.apply(&md, &opts(true)).unwrap().changed());
    assert!(!d.apply_model(&d.model(), &opts(true)).unwrap().changed());
    assert_eq!(d.dom.to_xml(), before);
    // Adding and removing formatting writes Word's properties.
    let edited = md.replace("<u>Defined Term</u>", "Defined Term").replace("Plain.", "<u>Under</u>, <mark data-color=\"cyan\">cyan</mark>, H<sub>2</sub>O, <span style=\"color:#0070C0\">blue</span>.");
    d.apply(&edited, &opts(false)).unwrap();
    let mut d = reopen(&mut d);
    let xml = d.dom.to_xml();
    assert!(xml.contains(r#"<w:highlight w:val="cyan"/>"#) && xml.contains(r#"<w:vertAlign w:val="subscript"/>"#) && xml.contains(r#"<w:color w:val="0070C0"/>"#), "{xml}");
    assert_eq!(xml.matches("<w:u ").count(), 1, "{xml}");
    let out = d.render(View::Review);
    assert!(out.contains(r#"<u>Under</u>, <mark data-color="cyan">cyan</mark>, H<sub>2</sub>O, <span style="color:#0070C0">blue</span>."#), "{out}");
    assert!(!d.apply(&out, &opts(false)).unwrap().changed());
}

#[test]
fn shading_and_raised_position_count_as_highlight_and_superscript() {
    // As LibreOffice writes them.
    let mut d = docx(r#"<w:p><w:r><w:t xml:space="preserve">20 m</w:t></w:r><w:r><w:rPr><w:position w:val="8"/><w:sz w:val="19"/></w:rPr><w:t>2</w:t></w:r><w:r><w:t xml:space="preserve">, </w:t></w:r><w:r><w:rPr><w:shd w:fill="FFFF00" w:val="clear"/></w:rPr><w:t>check</w:t></w:r></w:p>"#);
    let md = d.render(View::Review);
    assert!(md.contains("20 m<sup>2</sup>, <mark>check</mark>"), "{md}");
    assert!(!d.apply(&md, &opts(true)).unwrap().changed());
    // Removing them removes the shading and the raised position.
    d.apply(&md.replace("<sup>2</sup>", "2").replace("<mark>check</mark>", "check"), &opts(false)).unwrap();
    let xml = reopen(&mut d).dom.to_xml();
    assert!(!xml.contains("w:position") && !xml.contains("w:shd"), "{xml}");
}

#[test]
fn headers_and_footers_are_shown_read_only() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/header-footer.docx");
    let mut d = Docx::open(&path).unwrap();
    let md = d.render(View::Review);
    assert!(md.contains("<!-- docex:header ACME Corp · Confidential -->"), "{md}");
    assert!(md.contains("<!-- docex:footer Contract 2026-117 · Page 1 -->"), "{md}");
    assert!(!d.apply(&md, &opts(true)).unwrap().changed());
    // Editing them changes nothing, with a warning; the body edit still goes through.
    let r = d.apply(&md.replace("Confidential", "Public").replace("Body text", "The body text"), &opts(true)).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("read-only")), "{:?}", r.warnings);
    assert!(r.changed());
    assert!(d.render(View::Review).contains("ACME Corp · Confidential"));
    assert_eq!(d.model().furniture.len(), 2);
}

// A column chart as Word writes it (string categories, number caches, rich title).
const CHART_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Revenue by quarter</a:t></a:r></a:p></c:rich></c:tx><c:overlay val="0"/></c:title><c:autoTitleDeleted val="0"/><c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>2025</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:f>Sheet1!$A$2:$A$5</c:f><c:strCache><c:ptCount val="4"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt><c:pt idx="2"><c:v>Q3</c:v></c:pt><c:pt idx="3"><c:v>Q4</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!$B$2:$B$5</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="4"/><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>11</c:v></c:pt><c:pt idx="2"><c:v>15</c:v></c:pt><c:pt idx="3"><c:v>18</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:ser><c:idx val="1"/><c:order val="1"/><c:tx><c:strRef><c:f>Sheet1!$C$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>2026</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:f>Sheet1!$A$2:$A$5</c:f><c:strCache><c:ptCount val="4"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt><c:pt idx="2"><c:v>Q3</c:v></c:pt><c:pt idx="3"><c:v>Q4</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!$C$2:$C$5</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="4"/><c:pt idx="0"><c:v>12</c:v></c:pt><c:pt idx="1"><c:v>14</c:v></c:pt><c:pt idx="2"><c:v>19</c:v></c:pt><c:pt idx="3"><c:v>23.5</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;

const SMARTART_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dgm:ptLst><dgm:pt modelId="0" type="doc"><dgm:prSet/><dgm:t><a:bodyPr/><a:p><a:endParaRPr/></a:p></dgm:t></dgm:pt><dgm:pt modelId="1"><dgm:prSet/><dgm:t><a:bodyPr/><a:p><a:r><a:t>Plan</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="2"><dgm:t><a:bodyPr/><a:p><a:r><a:t>Build</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="3" type="parTrans"><dgm:t><a:p><a:r><a:t>ignored</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="4"><dgm:t><a:bodyPr/><a:p><a:r><a:t>Ship</a:t></a:r></a:p></dgm:t></dgm:pt></dgm:ptLst></dgm:dataModel>"#;

fn drawing(graphic: &str) -> String {
    format!(r#"<w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="5486400" cy="3200400"/><wp:docPr id="1" name="Chart 1"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData>{graphic}</a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#)
}

#[test]
fn charts_and_smartart_show_their_data() {
    let body = format!(
        "<w:p><w:r><w:t>Revenue grew.</w:t></w:r></w:p><w:p>{}</w:p><w:p>{}</w:p><w:p><w:r><w:t>After.</w:t></w:r></w:p>",
        drawing(r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rIdC1"/>"#),
        drawing(r#"<dgm:relIds xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:dm="rIdD1" r:lo="x" r:qs="y" r:cs="z"/>"#),
    );
    let mut d = docx_parts(&body, None, "", &[("word/charts/chart1.xml", "rIdC1", "chart", CHART_XML), ("word/diagrams/data1.xml", "rIdD1", "diagramData", SMARTART_XML)]);
    let md = d.render(View::Review);
    assert!(md.contains("<!--docex:chart rIdC1-->\n\n<!-- docex:chart rIdC1\nRevenue by quarter: column chart\n| | Q1 | Q2 | Q3 | Q4 |\n| 2025 | 10 | 11 | 15 | 18 |\n| 2026 | 12 | 14 | 19 | 23.5 |\n-->"), "{md}");
    assert!(md.contains("<!-- docex:smartart rIdD1\n- Plan\n- Build\n- Ship\n-->"), "{md}");
    let before = d.dom.to_xml();
    assert!(!d.apply(&md, &opts(true)).unwrap().changed());
    let m = d.model();
    assert_eq!(m.graphics.charts["<!--docex:chart rIdC1-->"].series[1].values[3], Some(23.5));
    assert!(!d.apply_model(&m, &opts(true)).unwrap().changed());
    assert_eq!(d.dom.to_xml(), before);
    // The data is read-only.
    let r = d.apply(&md.replace("| 2025 | 10 |", "| 2025 | 99 |").replace("After.", "After this."), &opts(true)).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("chart data")), "{:?}", r.warnings);
    assert!(d.render(View::Review).contains("| 2025 | 10 |"));
}

#[test]
fn format_changes_are_placed_in_the_text() {
    let mut d = fixture("rich.docx");
    let md = d.render(View::Review).replace("> A quoted paragraph.", "### A quoted paragraph.").replace("| Bob |", "| **Bob** |");
    let opts = ApplyOpts { author: "Ed".into(), date: "2026-10-30T00:00:00Z".into(), tracked: true, base_dir: None, keep_opaque: false };
    d.apply(&md, &opts).unwrap();
    let rv = d.review();
    let fmt: Vec<_> = rv.items.iter().filter(|i| i.author == "Ed").collect();
    assert_eq!(fmt.len(), 2, "{fmt:?}");
    for it in &fmt {
        assert_eq!(it.kind, "format");
        assert!(it.section.starts_with(&["Project Proposal".to_string(), "Goals".to_string()]), "{it:?}");
    }
    let texts: Vec<_> = fmt.iter().map(|i| i.text.clone().unwrap_or_default()).collect();
    assert!(texts.contains(&"Bob".to_string()) && texts.contains(&"A quoted paragraph.".to_string()), "{texts:?}");
    // The model carries the ids for editors.
    let m = d.model();
    let json = serde_json::to_string(&m).unwrap();
    assert!(json.contains("\"fmt\":") && json.contains("\"pfmt\":"), "model lacks format change ids");
}
