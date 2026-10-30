// Headless checks for diff, noise/clusters, "since", and applying AI edits.
import { EditorState, TextSelection } from "prosemirror-state";
import { applyEdits, markedModel, parseAdvice, parseEdits } from "../src/ai";
import { analyze } from "../src/analyze";
import { modelToDoc } from "../src/convert";
import { diffWords } from "../src/diff";
import { computeSince } from "../src/since";
import { assignAuthors, authorColor, authorPalette } from "../src/theme";
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

// AI edits: parsed from the answer, placed in the Markdown within the selection
const answer = `Here you go.
<edit>
<find>see the website</find>
<replace>see the [website](https://example.com)</replace>
</edit>
<edit all="true">
<find>ACME</find>
<replace>**ACME**</replace>
</edit>
<edit>
<find>not there</find>
<replace>x</replace>
</edit>`;
const { edits, note } = parseEdits(answer);
check("edits parsed", edits.length === 3 && edits[1].all && edits[0].replace === "see the [website](https://example.com)" && note === "Here you go.", JSON.stringify({ edits, note }));
const md = "# ACME\n\nFor details see the website of ACME.\n\nACME again, see the website.";
const region: [number, number] = [md.indexOf("For"), md.indexOf("ACME.") + 5];
const r = applyEdits(md, edits, region);
check("edits stay in the selection", r.md === "# ACME\n\nFor details see the [website](https://example.com) of **ACME**.\n\nACME again, see the website." && r.applied === 2 && r.failed.length === 1, JSON.stringify(r));
const twice = applyEdits(md, [{ find: "see the website", replace: "x", all: false }], null);
check("ambiguous edit refused", twice.applied === 0 && twice.failed.length === 1);
check("<none> gives the reason", parseEdits("<none>Already fine.</none>").note === "Already fine.");

// The selection is marked for the backend
let st = EditorState.create({ doc: modelToDoc({ blocks: [para([{ t: "One." }]), para([{ t: "Two words." }])] as DocModel["blocks"], boxes: [], notes: [] }) });
st = st.apply(st.tr.setSelection(TextSelection.create(st.doc, 7, 10)));
const marked = JSON.stringify(markedModel(st).blocks.map((b) => (b as { spans: { t?: string }[] }).spans.map((x) => x.t).join("")));
check("selection marked", marked === JSON.stringify(["One.", "\uE000Two\uE001 words."]), marked);

// AI advice for triage: by number, unknown numbers and malformed lines ignored
const advice = parseAdvice('<advice id="2" do="reject">Contradicts the table.</advice>\n<advice id="1" do="accept">Typo fix.</advice>\n<advice id="9" do="accept">x</advice>\n<advice id="3" do="maybe">?</advice>', idx.items.slice(0, 3));
check("advice parsed", advice.size === 2 && advice.get(idx.items[1].key)?.do === "reject" && advice.get(idx.items[0].key)?.why === "Typo fix.", JSON.stringify([...advice]));

// Author colours: distinct, and handed out in order of appearance
const tokyo = { accent: "#7aa2f7", background: "#1a1b26", red: "#f7768e", yellow: "#e0af68", orange: "#ff9e64", green: "#9ece6a", cyan: "#7dcfff", blue: "#7aa2f7", magenta: "#bb9af7" };
const pal = authorPalette(tokyo);
check("palette: theme hues first, accent = blue once", JSON.stringify(pal.slice(0, 5)) === JSON.stringify(["#7aa2f7", "#bb9af7", "#ff9e64", "#7dcfff", "#e0af68"]), JSON.stringify(pal));
check("palette: a dozen or more colours", pal.length >= 12 && new Set(pal).size === pal.length, String(pal.length));
const slot = (n: string) => authorColor(n).match(/--au(\d+)/)![1];
assignAuthors(["Me", "Anna", "Bob", "Anna", "Cleo"]);
check("first people get different colours", [slot("Me"), slot("Anna"), slot("Bob"), slot("Cleo")].join() === "0,1,2,3");
assignAuthors(["Me", "Dora", "Cleo"]);
check("colours stay; newcomers take a free one", slot("Cleo") === "3" && slot("Dora") === "1", `${slot("Cleo")} ${slot("Dora")}`);
process.exit(failed ? 1 : 0);
