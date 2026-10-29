//! Debug helper: apply a model JSON to a document and print the Markdown review view.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut d = docex_core::Docx::open(a[0].as_ref()).unwrap();
    let m = serde_json::from_str(&std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    let opts = docex_core::ApplyOpts { author: "Me".into(), date: "2026-01-01T00:00:00Z".into(), tracked: true, base_dir: None, keep_opaque: false };
    let r = d.apply_model(&m, &opts).unwrap();
    eprintln!("{:?}", r.stats);
    print!("{}", d.render(docex_core::render::View::Review));
}
