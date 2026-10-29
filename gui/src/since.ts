// "Since you last saw it": highlight what differs from the text as it was the
// last time this document (or the file it was renamed from) was opened or
// saved in docex. Works whether or not the others tracked their changes.

import type { Node as PMNode } from "prosemirror-model";
import { Plugin, PluginKey } from "prosemirror-state";
import { Decoration, DecorationSet, type EditorView } from "prosemirror-view";
import { diffWords } from "./diff";
import { schema } from "./schema";

export interface Snapshot {
  path: string;
  name: string;
  taken: string;
  paras: string[];
}

export interface SinceResult {
  changed: number;
  added: number;
  removed: number;
  decos: DecorationSet;
  /** Document positions of differences, for jumping. */
  spots: number[];
}

interface Para {
  text: string;
  /** Document position of each character of `text`. */
  at: number[];
  pos: number;
  node: PMNode;
}

/** Paragraphs in reading order, as accepted text (deletions left out). */
function paras(doc: PMNode): Para[] {
  const out: Para[] = [];
  doc.descendants((node, pos) => {
    if (node.type !== schema.nodes.para) return true;
    let text = "";
    const at: number[] = [];
    node.forEach((child, off) => {
      if (schema.marks.del.isInSet(child.marks)) return;
      const start = pos + 1 + off;
      if (child.isText) {
        for (let k = 0; k < child.text!.length; k++) at.push(start + k);
        text += child.text;
      } else {
        at.push(start);
        text += "￼";
      }
    });
    if (text.trim()) out.push({ text, at, pos, node });
    return false;
  });
  return out;
}

export function paraTexts(doc: PMNode): string[] {
  return paras(doc).map((p) => p.text);
}

function similarity(a: string, b: string): number {
  const ta = new Set(a.toLowerCase().split(/\s+/).filter(Boolean));
  const tb = new Set(b.toLowerCase().split(/\s+/).filter(Boolean));
  if (!ta.size || !tb.size) return 0;
  let common = 0;
  for (const t of ta) if (tb.has(t)) common++;
  return common / Math.max(ta.size, tb.size);
}

/** LCS alignment of two string lists: pairs of equal indices, in order. */
function lcs(a: string[], b: string[]): [number, number][] {
  const n = a.length;
  const m = b.length;
  if (n * m > 25_000_000) return [];
  const L = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) L[i][j] = a[i] === b[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
  const out: [number, number][] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push([i, j]);
      i++;
      j++;
    } else if (L[i + 1][j] >= L[i][j + 1]) i++;
    else j++;
  }
  return out;
}

function removedWidget(text: string) {
  return () => {
    const el = document.createElement("div");
    el.className = "since-removed";
    el.textContent = text;
    el.title = "Removed since you last saw it";
    return el;
  };
}

export function computeSince(doc: PMNode, prev: Snapshot): SinceResult {
  const cur = paras(doc);
  const old = prev.paras.filter((p) => p.trim());
  const pairs = lcs(old, cur.map((p) => p.text));
  const decos: Decoration[] = [];
  const spots: number[] = [];
  let changed = 0;
  let added = 0;
  let removed = 0;
  // Walk the gaps between equal paragraphs.
  const bounds: [number, number][] = [...pairs, [old.length, cur.length]];
  let oi = 0;
  let ci = 0;
  for (const [oe, ce] of bounds) {
    const gapOld = old.slice(oi, oe);
    const gapCur = cur.slice(ci, ce);
    const usedOld = new Set<number>();
    for (const c of gapCur) {
      // Pair with the most similar unused old paragraph in this gap.
      let best = -1;
      let bestS = 0.35;
      gapOld.forEach((o, k) => {
        if (usedOld.has(k)) return;
        const s = similarity(o, c.text);
        if (s > bestS) {
          bestS = s;
          best = k;
        }
      });
      if (best < 0) {
        added++;
        spots.push(c.pos);
        decos.push(Decoration.node(c.pos, c.pos + c.node.nodeSize, { class: "since-new" }));
        continue;
      }
      usedOld.add(best);
      changed++;
      spots.push(c.pos);
      let k = 0;
      for (const op of diffWords(gapOld[best], c.text)) {
        if (op.type === "eq") k += op.text.length;
        else if (op.type === "ins") {
          const from = c.at[k];
          const to = (c.at[k + op.text.length - 1] ?? from) + 1;
          if (from !== undefined) decos.push(Decoration.inline(from, to, { class: "since-ins", title: "New since you last saw it" }));
          k += op.text.length;
        } else if (op.text.trim()) {
          const at = c.at[k] ?? (c.at[k - 1] !== undefined ? c.at[k - 1] + 1 : c.pos + 1);
          decos.push(
            Decoration.widget(at, () => {
              const el = document.createElement("span");
              el.className = "since-del";
              el.textContent = op.text;
              el.title = "Removed since you last saw it";
              return el;
            }, { side: -1, ignoreSelection: true }),
          );
        }
      }
    }
    // Old paragraphs with no counterpart were removed.
    const anchor = ci > 0 ? cur[ci - 1].pos + cur[ci - 1].node.nodeSize : 0;
    gapOld.forEach((o, k) => {
      if (usedOld.has(k)) return;
      removed++;
      decos.push(Decoration.widget(anchor, removedWidget(o), { side: 1 }));
    });
    oi = oe + 1;
    ci = ce + 1;
  }
  return { changed, added, removed, decos: DecorationSet.create(doc, decos), spots: spots.sort((a, b) => a - b) };
}

const key = new PluginKey<{ set: DecorationSet; on: boolean }>("docex-since");

export const sincePlugin = new Plugin<{ set: DecorationSet; on: boolean }>({
  key,
  state: {
    init: () => ({ set: DecorationSet.empty, on: false }),
    apply(tr, st) {
      const m = tr.getMeta(key) as { set?: DecorationSet; on?: boolean } | undefined;
      const set = m?.set ?? st.set.map(tr.mapping, tr.doc);
      return { set, on: m?.on ?? st.on };
    },
  },
  props: {
    decorations(state) {
      const st = key.getState(state);
      return st?.on ? st.set : DecorationSet.empty;
    },
  },
});

export function setSince(view: EditorView, set: DecorationSet | undefined, on: boolean) {
  view.dispatch(view.state.tr.setMeta(key, { set, on }).setMeta("addToHistory", false));
}
