// Show edits as tracked changes while typing (like Word with Track Changes on).
//
// This is only for display: the docex core recomputes the real tracked changes
// against the document when the editor syncs. Text typed with tracking on is
// marked as an insertion; deleted text is struck through instead of removed,
// unless it is your own insertion, which simply disappears.

import type { EditorState, Transaction } from "prosemirror-state";
import { TextSelection } from "prosemirror-state";
import { ReplaceStep } from "prosemirror-transform";
import { schema } from "./schema";

export function trackTransaction(state: EditorState, tr: Transaction, author: string): Transaction | null {
  if (!tr.docChanged || tr.getMeta("docex-raw") || tr.steps.length !== 1) return null;
  const step = tr.steps[0];
  if (!(step instanceof ReplaceStep)) return null;
  const { from, to, slice } = step as unknown as { from: number; to: number; slice: import("prosemirror-model").Slice };
  // Splitting or joining paragraphs is structural; let it through.
  if (slice.openStart > 0 || slice.openEnd > 0) return null;
  if (to > from && !state.doc.resolve(from).sameParent(state.doc.resolve(to))) return null;

  const { ins, del } = schema.marks;
  const insMark = ins.create({ author, date: "", wid: "" });
  const delMark = del.create({ author, date: "", wid: "" });
  const t = state.tr;

  // Deleted range: strike through, except our own insertions, which vanish.
  const drop: [number, number][] = [];
  if (to > from) {
    state.doc.nodesBetween(from, to, (node, pos) => {
      if (!node.isInline) return true;
      const s = Math.max(pos, from);
      const e = Math.min(pos + node.nodeSize, to);
      if (e <= s) return false;
      const i = ins.isInSet(node.marks);
      if (i && (!i.attrs.wid || i.attrs.author === author)) drop.push([s, e]);
      else if (!del.isInSet(node.marks)) t.addMark(s, e, delMark);
      return false;
    });
    for (const [s, e] of drop.reverse()) t.delete(s, e);
  }

  // Inserted content goes after the struck-through text.
  let caret: number;
  if (slice.size > 0) {
    const at = t.mapping.map(to);
    const before = t.doc.content.size;
    t.replace(at, at, slice);
    const end = at + (t.doc.content.size - before);
    t.removeMark(at, end, del);
    t.removeMark(at, end, ins);
    t.addMark(at, end, insMark);
    caret = end;
  } else {
    // Backspace leaves the caret before the deleted text, Delete after it.
    const backspace = state.selection.empty && state.selection.head === to;
    caret = backspace || !state.selection.empty ? t.mapping.map(from, -1) : t.mapping.map(to, 1);
  }
  t.setSelection(TextSelection.create(t.doc, Math.min(caret, t.doc.content.size)));
  for (const k of ["addToHistory", "paste", "uiEvent"]) {
    const v = tr.getMeta(k);
    if (v !== undefined) t.setMeta(k, v);
  }
  if (tr.scrolledIntoView) t.scrollIntoView();
  return t;
}
