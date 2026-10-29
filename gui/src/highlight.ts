// Temporary highlights in the document (hovered or focused review items).
//
// These must be decorations: changing the editor's DOM directly (e.g. adding
// a class) is read by ProseMirror as an edit of the text.

import { Plugin, PluginKey } from "prosemirror-state";
import { Decoration, DecorationSet, type EditorView } from "prosemirror-view";

export interface HlRange {
  from: number;
  to: number;
}

const key = new PluginKey<DecorationSet>("docex-highlight");

export const highlightPlugin = new Plugin<DecorationSet>({
  key,
  state: {
    init: () => DecorationSet.empty,
    apply(tr, set) {
      const ranges = tr.getMeta(key) as HlRange[] | null | undefined;
      if (ranges === undefined) return set.map(tr.mapping, tr.doc);
      if (!ranges) return DecorationSet.empty;
      const max = tr.doc.content.size;
      const decos = ranges
        .map((r) => ({ from: Math.max(0, Math.min(r.from, max)), to: Math.max(0, Math.min(r.to, max)) }))
        .filter((r) => r.to > r.from)
        .map((r) => Decoration.inline(r.from, r.to, { class: "hl" }));
      return DecorationSet.create(tr.doc, decos);
    },
  },
  props: {
    decorations: (state) => key.getState(state),
  },
});

export function setHighlight(view: EditorView, ranges: HlRange[] | null) {
  const cur = key.getState(view.state);
  if (!ranges && (!cur || cur === DecorationSet.empty)) return;
  view.dispatch(view.state.tr.setMeta(key, ranges).setMeta("addToHistory", false));
}
