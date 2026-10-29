// Markdown shortcuts: type text character by character, as the input-rule plugin sees it.
import { EditorState, TextSelection } from "prosemirror-state";
import type { InputRule } from "prosemirror-inputrules";
import { docToModel } from "../src/convert";
import { markdownRuleList } from "../src/markdown";
import { schema } from "../src/schema";

const rules = markdownRuleList() as unknown as (InputRule & { match: RegExp; handler: (...a: unknown[]) => import("prosemirror-state").Transaction | null })[];

function type(text: string): EditorState {
  let state = EditorState.create({ doc: schema.nodes.doc.create(null, schema.nodes.para.create()) });
  state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, 1)));
  for (const ch of text) {
    const { $from, from } = state.selection;
    const before = $from.parent.textBetween(0, $from.parentOffset, undefined, "￼") + ch;
    let done = false;
    for (const r of rules) {
      const m = r.match.exec(before);
      if (!m) continue;
      const tr = r.handler(state, m, from - (m[0].length - ch.length), from);
      if (tr) {
        state = state.apply(tr);
        done = true;
        break;
      }
    }
    if (!done) state = state.apply(state.tr.insertText(ch));
  }
  return state;
}

const cases: [string, string][] = [
  ["## Title", '{"kind":"h2","level":0,"spans":[{"t":"Title"}]}'],
  ["# A", '{"kind":"h1","level":0,"spans":[{"t":"A"}]}'],
  ["- item", '{"kind":"bullet","level":0,"spans":[{"t":"item"}]}'],
  ["  * sub", '{"kind":"bullet","level":1,"spans":[{"t":"sub"}]}'],
  ["1. one", '{"kind":"number","level":0,"spans":[{"t":"one"}]}'],
  ["> q", '{"kind":"quote","level":0,"spans":[{"t":"q"}]}'],
  ["```x", '{"kind":"code","level":0,"spans":[{"t":"x"}]}'],
  ["a **bold** b", '{"kind":"normal","level":0,"spans":[{"t":"a "},{"t":"bold","b":true},{"t":" b"}]}'],
  ["a *it* b", '{"kind":"normal","level":0,"spans":[{"t":"a "},{"t":"it","i":true},{"t":" b"}]}'],
  ["x `c` y", '{"kind":"normal","level":0,"spans":[{"t":"x "},{"t":"c","c":true},{"t":" y"}]}'],
  ["~~gone~~", '{"kind":"normal","level":0,"spans":[{"t":"gone","s":true}]}'],
  ["see [docs](https://x.org) now", '{"kind":"normal","level":0,"spans":[{"t":"see "},{"t":"docs","link":"https://x.org"},{"t":" now"}]}'],
  ["snake_case_name_ ok", '{"kind":"normal","level":0,"spans":[{"t":"snake_case_name_ ok"}]}'],
  ["3 * 4 * 5", '{"kind":"normal","level":0,"spans":[{"t":"3 * 4 * 5"}]}'],
  ["E $E=mc^2$ ok", '{"kind":"normal","level":0,"spans":[{"t":"E "},{"obj":"math:E=mc^2"},{"t":" ok"}]}'],
  ["$$\\int_0^1 x$$", '{"kind":"normal","level":0,"spans":[{"obj":"mathd:\\\\int_0^1 x"}]}'],
  ["costs $5 and $10", '{"kind":"normal","level":0,"spans":[{"t":"costs $5 and $10"}]}'],
  ["Price: 5 # not heading", '{"kind":"normal","level":0,"spans":[{"t":"Price: 5 # not heading"}]}'],
];
const canon = (v: unknown): unknown =>
  Array.isArray(v) ? v.map(canon) : v && typeof v === "object" ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => a.localeCompare(b)).map(([k, x]) => [k, canon(x)])) : v;
let failed = 0;
for (const [input, want] of cases) {
  const b = docToModel(type(input).doc).blocks[0];
  const { type: _t, ...rest } = b as Record<string, unknown>;
  const got = JSON.stringify(canon(rest));
  const ok = got === JSON.stringify(canon(JSON.parse(want)));
  if (!ok) failed++;
  console.log(`${ok ? "ok  " : "FAIL"} ${JSON.stringify(input)}${ok ? "" : `\n     got  ${got}\n     want ${want}`}`);
}
process.exit(failed ? 1 : 0);
