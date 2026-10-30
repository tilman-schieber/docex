// Table structure edits as in the editor (merge cells, add a column); prints
// the model for the core to apply.
import { readFileSync, writeFileSync } from "node:fs";
import { EditorState, TextSelection } from "prosemirror-state";
import { addColumnAfter, CellSelection, mergeCells } from "prosemirror-tables";
import { docToModel, modelToDoc } from "../src/convert";

const [input, output] = process.argv.slice(2);
let state = EditorState.create({ doc: modelToDoc(JSON.parse(readFileSync(input, "utf8"))) });
const run = (cmd: (s: EditorState, d: (tr: import("prosemirror-state").Transaction) => void) => boolean) => {
  if (!cmd(state, (tr) => (state = state.apply(tr)))) throw new Error("command not applicable");
};
// Position of the first table cell whose text is `text`.
const cell = (text: string) => {
  let at = -1;
  state.doc.descendants((n, pos) => {
    if (at < 0 && n.type.name === "table_cell" && n.textContent === text) at = pos;
    return at < 0;
  });
  if (at < 0) throw new Error(`no cell ${text}`);
  return at;
};
// 1. Select "Staff" and "Material" (under 2026) and merge them.
state = state.apply(state.tr.setSelection(CellSelection.create(state.doc, cell("Staff"), cell("Material"))));
run(mergeCells);
// 2. With the cursor in "WP1 Data pipeline", add a column to its right.
state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, cell("WP1 Data pipeline") + 2)));
run(addColumnAfter);
writeFileSync(output, JSON.stringify(docToModel(state.doc)));
