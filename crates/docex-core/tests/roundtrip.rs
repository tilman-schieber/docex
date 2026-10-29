use docex_core::render::View;
use docex_core::{ApplyOpts, Docx};
use std::io::{Read, Write};

/// A docx whose body is `body`, built on the blank template.
fn docx(body: &str) -> Docx {
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
        zout.start_file(name, opts).unwrap();
        zout.write_all(data.as_bytes()).unwrap();
    }
    Docx::from_bytes(zout.finish().unwrap().into_inner()).unwrap()
}

fn opts(tracked: bool) -> ApplyOpts {
    ApplyOpts { author: "Me".into(), date: "2026-10-29T00:00:00Z".into(), tracked, base_dir: None }
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
    assert!(md.contains("The tenant pays {--500--}{++600++} euros per month."), "{md}");
    assert_eq!(d.render(View::Accepted).lines().nth(2), Some("The tenant pays 600 euros per month."));
    assert_eq!(d.render(View::Original).lines().nth(2), Some("The tenant pays 500 euros per month."));
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
    assert!(d2.render(View::Review).contains("The tenant always pays 600 euros per calendar month."));
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
    assert!(d.render(View::Review).contains("pays {--500--}600 euros"));
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
            MBlock::Table { rows } => format!("table{}", rows.len()),
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
    let md = d.render(View::Review);
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
