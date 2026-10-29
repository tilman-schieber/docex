// Simulated typing with tracking on; prints the model for the core to apply.
import { readFileSync, writeFileSync } from "node:fs";
import { EditorState, TextSelection } from "prosemirror-state";
import { docToModel, modelToDoc } from "../src/convert";
import { trackTransaction } from "../src/track";

const [input, output] = process.argv.slice(2);
let state = EditorState.create({ doc: modelToDoc(JSON.parse(readFileSync(input, "utf8"))) });
const apply = (tr: import("prosemirror-state").Transaction) => {
  state = state.apply(trackTransaction(state, tr, "Me") ?? tr);
};
// Find a word in the first paragraph containing it.
const find = (word: string) => {
  let at = -1;
  state.doc.descendants((n, pos) => {
    if (at < 0 && n.isText && n.text!.includes(word)) at = pos + n.text!.indexOf(word);
  });
  if (at < 0) throw new Error(`no ${word}`);
  return at;
};
// 1. Type " brave" after "Second".
let p = find("Second") + "Second".length;
state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, p)));
for (const ch of " brave") apply(state.tr.insertText(ch));
// 2. Backspace twice: removes our own "ve" for real.
for (let i = 0; i < 2; i++) {
  const h = state.selection.head;
  apply(state.tr.delete(h - 1, h));
}
// 3. Select "stays" and type "remains": "stays" struck through, "remains" inserted.
p = find("stays");
state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, p, p + 5)));
apply(state.tr.insertText("r"));
for (const ch of "emains") apply(state.tr.insertText(ch));
writeFileSync(output, JSON.stringify(docToModel(state.doc)));
const para = docToModel(state.doc).blocks.find((b) => b.type === "para" && b.spans.some((s) => s.t?.includes("Second")));
console.log(JSON.stringify(para && "spans" in para ? para.spans : para));
