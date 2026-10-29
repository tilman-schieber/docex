//! Debug helper for the GUI model.
//!   modelcheck dump FILE.docx OUT.json   write the document's model
//!   modelcheck check FILE.docx IN.json   apply a model; report whether anything changed
//!   modelcheck FILE.docx...              model -> JSON -> apply must be a no-op
fn opts() -> docex_core::ApplyOpts {
    docex_core::ApplyOpts { author: "X".into(), date: "2026-01-01T00:00:00Z".into(), tracked: true, base_dir: None, keep_opaque: false }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("dump") => {
            let d = docex_core::Docx::open(args[1].as_ref()).unwrap();
            std::fs::write(&args[2], serde_json::to_string(&d.model()).unwrap()).unwrap();
        }
        Some("check") => {
            let mut d = docex_core::Docx::open(args[1].as_ref()).unwrap();
            let before = d.dom.to_xml();
            let m = serde_json::from_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
            let r = d.apply_model(&m, &opts()).unwrap();
            println!("{} :: changed={} identical={} {:?}", args[1].rsplit('/').next().unwrap(), r.changed(), d.dom.to_xml() == before, r.warnings);
        }
        _ => {
            for path in &args {
                let mut d = docex_core::Docx::open(path.as_ref()).unwrap();
                let before = d.dom.to_xml();
                let json = serde_json::to_string(&d.model()).unwrap();
                let r = d.apply_model(&serde_json::from_str(&json).unwrap(), &opts()).unwrap();
                println!("{} :: changed={} identical={}", path.rsplit('/').next().unwrap(), r.changed(), d.dom.to_xml() == before);
            }
        }
    }
}
