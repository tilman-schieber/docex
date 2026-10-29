//! Debug helper: show where a document's rendering stops reparsing 1:1.
use docex_core::mdparse::{parse, EBlock};
use docex_core::model::Block;
use docex_core::render::View;

fn main() {
    let path = std::env::args().nth(1).expect("usage: blockdiff FILE.docx");
    let d = docex_core::Docx::open(path.as_ref()).unwrap();
    let blocks = d.blocks();
    let md = d.render(View::Review);
    let parsed = parse(&md);
    let rendered: Vec<String> = blocks
        .iter()
        .filter_map(|b| match b {
            Block::Para(p) if !p.is_empty() => Some(format!("P {:?}: {}", p.kind, p.plain_text().chars().take(60).collect::<String>())),
            Block::Para(_) => None,
            Block::Table(t) => Some(format!("T b{}", t.idx)),
            Block::Opaque { idx, .. } => Some(format!("O b{idx}")),
        })
        .collect();
    let got: Vec<String> = parsed
        .iter()
        .map(|b| match b {
            EBlock::Para(p) => format!("P {:?}: {}", p.kind, p.text().chars().take(60).collect::<String>()),
            EBlock::Table(_) => "T".into(),
            EBlock::Opaque(n) => format!("O b{n}"),
        })
        .collect();
    for (i, (a, b)) in rendered.iter().zip(&got).enumerate() {
        let same = a.split(':').next() == b.split(':').next() || (a.starts_with('T') && (b == "T" || b.starts_with('O')));
        if !same {
            println!("first mismatch at {i}:\n  doc:    {a}\n  parsed: {b}");
            for k in i.saturating_sub(2)..(i + 3).min(got.len()) {
                println!("  ctx {k}: {} || {}", rendered.get(k).map(String::as_str).unwrap_or(""), got[k]);
            }
            return;
        }
    }
    println!("{} vs {} blocks, no kind mismatch in common prefix", rendered.len(), got.len());
}
