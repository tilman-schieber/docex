// Headless checks for diff, noise/clusters, "since", and AI rewrite application.
import { EditorState, TextSelection } from "prosemirror-state";
import { rewriteTransaction, selectionParas } from "../src/ai";
import { analyze } from "../src/analyze";
import { docToModel, modelToDoc } from "../src/convert";
import { diffWords } from "../src/diff";
import { computeSince } from "../src/since";
import type { DocModel, View } from "../src/types";

let failed = 0;
const check = (name: string, ok: boolean, detail = "") => {
  if (!ok) failed++;
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${ok ? "" : `\n     ${detail}`}`);
};

// diff
const d = diffWords("The tenant pays 500 euros", "The tenant always pays 600 euros");
check("word diff", JSON.stringify(d) === JSON.stringify([{ type: "eq", text: "The tenant " }, { type: "ins", text: "always " }, { type: "eq", text: "pays " }, { type: "del", text: "500" }, { type: "ins", text: "600" }, { type: "eq", text: " euros" }]), JSON.stringify(d));

// noise and clusters
const para = (spans: object[], kind = "normal") => ({ type: "para", kind, level: 0, spans });
const m: DocModel = {
  blocks: [
    para([{ t: "Intro" }], "h1"),
    para([{ t: "We use " }, { t: "KI", st: "del", author: "A", wid: "1" }, { t: "AI", st: "ins", author: "A", wid: "2" }, { t: " daily" }, { t: ",", st: "ins", author: "B", wid: "3" }]),
    para([{ t: "More " }, { t: "KI", st: "del", author: "A", wid: "4" }, { t: "AI", st: "ins", author: "A", wid: "5" }, { t: " here." }]),
  ] as DocModel["blocks"],
  boxes: [],
  notes: [],
};
const kinds = ["", "delete", "insert", "insert", "delete", "insert"];
const changes = [1, 2, 3, 4, 5].map((n) => ({ n, kind: kinds[n], author: n === 3 ? "B" : "A", date: "", text: "", part: "body", wids: [String(n)] }));
const view = { changes, comments: [] } as unknown as View;
const idx = analyze(modelToDoc(m), view, "Me");
check("replacements merged", idx.items.filter((i) => i.kind === "replace").length === 2, JSON.stringify(idx.items.map((i) => i.kind)));
check("cluster found", idx.clusters.length === 1 && idx.clusters[0].items.length === 2 && idx.clusters[0].old === "KI" && idx.clusters[0].new === "AI", JSON.stringify(idx.clusters));
check("punctuation is noise", idx.items.find((i) => i.new === ",")?.noise === "punctuation" && idx.totals.noise === 1);
check("section assigned", idx.items.every((i) => i.section === 1));

// since
const docNow = modelToDoc({ blocks: [para([{ t: "Same paragraph stays." }]), para([{ t: "The tenant always pays 600 euros." }]), para([{ t: "Brand new paragraph here." }])] as DocModel["blocks"], boxes: [], notes: [] });
const s = computeSince(docNow, { path: "x", name: "x", taken: "", paras: ["Same paragraph stays.", "The tenant pays 500 euros.", "A paragraph that was removed."] });
check("since counts", s.changed === 1 && s.added === 1 && s.removed === 1, JSON.stringify({ c: s.changed, a: s.added, r: s.removed }));

// AI rewrite -> minimal tracked changes
let st = EditorState.create({ doc: modelToDoc({ blocks: [para([{ t: "The tenant pays 500 euros per month." }])] as DocModel["blocks"], boxes: [], notes: [] }) });
st = st.apply(st.tr.setSelection(TextSelection.create(st.doc, 1, 1 + "The tenant pays 500 euros per month.".length)));
const sel = selectionParas(st);
if (typeof sel === "string") check("selection", false, sel);
else {
  const { tr, edits } = rewriteTransaction(st, sel.paras, ["The tenant pays 600 euros monthly."], "Me");
  const spans = (docToModel(tr.doc).blocks[0] as { spans: { t?: string; st?: string }[] }).spans.map((x) => (x.st ? `${x.st}:${x.t}` : x.t)).join("|");
  check("rewrite is minimal and tracked", spans === "The tenant pays |del:500|ins:600| euros |del:per month|ins:monthly|.", `${spans} (${edits} edits)`);
}
process.exit(failed ? 1 : 0);
