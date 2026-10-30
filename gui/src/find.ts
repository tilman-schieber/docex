// Find and replace in the document. Matches are decorations; replacing goes
// through the same live tracking as typing, so with Track Changes on every
// replacement is a tracked change. "Replace all" is one undo step.

import { Plugin, PluginKey, TextSelection, type EditorState, type Transaction } from "prosemirror-state";
import type { Node as PMNode } from "prosemirror-model";
import { Decoration, DecorationSet, type EditorView } from "prosemirror-view";
import { schema } from "./schema";
import { trackTransaction } from "./track";

export interface Query {
  text: string;
  caseSensitive: boolean;
  wholeWord: boolean;
}

interface Match {
  from: number;
  to: number;
}

interface FindState {
  query: Query | null;
  matches: Match[];
  current: number;
}

const key = new PluginKey<FindState>("docex-find");

const isWordChar = (c?: string) => !!c && /[\p{L}\p{N}_]/u.test(c);

/** Matches of `q` within each paragraph; struck-through (deleted) text and objects never match. */
export function findMatches(doc: PMNode, q: Query): Match[] {
  const out: Match[] = [];
  if (!q.text) return out;
  // Folded per UTF-16 unit, so indices in the folded text match the original.
  const fold = (c: string) => (q.caseSensitive ? c : c.toLowerCase().length === 1 ? c.toLowerCase() : c);
  const needle = q.text.split("").map(fold).join("");
  doc.descendants((node, pos) => {
    if (!node.isTextblock) return true;
    let s = "";
    const at: number[] = [];
    node.forEach((child, off) => {
      const start = pos + 1 + off;
      if (child.isText && !schema.marks.del.isInSet(child.marks)) {
        const t = child.text!;
        for (let i = 0; i < t.length; i++) {
          s += t[i];
          at.push(start + i);
        }
      } else {
        // One blocker per deleted character or object, so positions stay aligned.
        for (let i = 0; i < child.nodeSize; i++) {
          s += "\u0000";
          at.push(start + i);
        }
      }
    });
    const hay = s.split("").map(fold).join("");
    for (let i = hay.indexOf(needle); i >= 0; i = hay.indexOf(needle, i + 1)) {
      const j = i + needle.length;
      if (q.wholeWord && (isWordChar(s[i - 1]) || isWordChar(s[j]))) continue;
      out.push({ from: at[i], to: at[j - 1] + 1 });
    }
    return false;
  });
  return out;
}

export const findPlugin = new Plugin<FindState>({
  key,
  state: {
    init: () => ({ query: null, matches: [], current: -1 }),
    apply(tr, st, _old, state) {
      const meta = tr.getMeta(key) as Partial<FindState> | undefined;
      let next = meta ? { ...st, ...meta } : st;
      if (next.query && (tr.docChanged || meta?.query !== undefined)) {
        const matches = findMatches(state.doc, next.query);
        let current = meta?.current ?? next.current;
        if (meta?.current === undefined) {
          // Keep to the match at (or after) where the current one was.
          const was = tr.mapping.map(st.matches[st.current]?.from ?? state.selection.from);
          current = matches.findIndex((m) => m.to > was);
          if (current < 0) current = matches.length ? 0 : -1;
        }
        next = { ...next, matches, current: Math.min(current, matches.length - 1) };
      }
      if (!next.query) next = { query: null, matches: [], current: -1 };
      return next;
    },
  },
  props: {
    decorations(state) {
      const st = key.getState(state);
      if (!st?.query || !st.matches.length) return null;
      return DecorationSet.create(
        state.doc,
        st.matches.map((m, i) => Decoration.inline(m.from, m.to, { class: i === st.current ? "find-hit cur" : "find-hit" })),
      );
    },
  },
});

export function findState(state: EditorState): FindState {
  return key.getState(state) ?? { query: null, matches: [], current: -1 };
}

function show(view: EditorView, i: number) {
  const m = findState(view.state).matches[i];
  const tr = view.state.tr.setMeta(key, { current: i }).setMeta("addToHistory", false);
  if (m) tr.setSelection(TextSelection.create(tr.doc, m.from, m.to)).scrollIntoView();
  view.dispatch(tr);
}

/** Search for `q` (null: stop), starting at the selection. */
export function setQuery(view: EditorView, q: Query | null) {
  view.dispatch(view.state.tr.setMeta(key, { query: q, current: undefined }).setMeta("addToHistory", false));
  const st = findState(view.state);
  if (q && st.current >= 0) show(view, st.current);
}

/** Go to the next (1) or previous (-1) match. */
export function step(view: EditorView, d: number) {
  const st = findState(view.state);
  if (!st.matches.length) return;
  show(view, (st.current + d + st.matches.length) % st.matches.length);
}

/** Replace the current match; the next one becomes current. */
export function replaceCurrent(view: EditorView, text: string) {
  const st = findState(view.state);
  const m = st.matches[st.current];
  if (!m) return;
  // An ordinary edit: dispatchTransaction tracks it when Track Changes is on.
  view.dispatch(view.state.tr.insertText(text, m.from, m.to));
  const after = findState(view.state);
  if (after.matches.length) show(view, Math.min(after.current, after.matches.length - 1));
}

/** Replace every match in one step (tracked one by one when `tracking`). Returns the count. */
export function replaceAll(view: EditorView, text: string, tracking: boolean, author: string): number {
  const matches = findState(view.state).matches;
  if (!matches.length) return 0;
  let state = view.state;
  const all: Transaction = view.state.tr;
  // Back to front, so earlier positions stay valid; each replacement is tracked
  // against the state before it, and their steps are collected into one transaction.
  for (const m of [...matches].reverse()) {
    const one = state.tr.insertText(text, m.from, m.to);
    const t = tracking ? (trackTransaction(state, one, author) ?? one) : one;
    for (const s of t.steps) all.step(s);
    state = state.apply(t);
  }
  view.dispatch(all.setMeta("docex-raw", true).scrollIntoView());
  return matches.length;
}

export interface FindHost {
  tracking(): boolean;
  author(): string;
  toast(msg: string): void;
}

/** The find bar over the top right of the page: Ctrl+F finds, Ctrl+H also replaces. */
export function createFindBar(view: EditorView, host: FindHost) {
  const el = document.createElement("div");
  el.className = "findbar";
  el.hidden = true;
  el.innerHTML = `
    <div class="fb-row">
      <input class="fb-find" placeholder="Find" spellcheck="false" />
      <span class="fb-count"></span>
      <button class="fb-prev" data-tip="Previous  Shift+Enter">↑</button>
      <button class="fb-next" data-tip="Next  Enter">↓</button>
      <button class="fb-case" data-tip="Match case">Aa</button>
      <button class="fb-word" data-tip="Whole words">W</button>
      <button class="fb-close" data-tip="Close  Esc">×</button>
    </div>
    <div class="fb-row fb-replace-row">
      <input class="fb-replace" placeholder="Replace with" spellcheck="false" />
      <button class="fb-one" data-tip="Replace this one  Enter">Replace</button>
      <button class="fb-all" data-tip="Replace all (one undo step)  Ctrl+Enter">All</button>
    </div>`;
  document.getElementById("page-area")!.append(el);
  const q = <T extends HTMLElement>(c: string) => el.querySelector(`.${c}`) as T;
  const find = q<HTMLInputElement>("fb-find");
  const repl = q<HTMLInputElement>("fb-replace");
  const count = q<HTMLSpanElement>("fb-count");
  const opts = { caseSensitive: false, wholeWord: false };

  const query = (): Query | null => (find.value ? { text: find.value, ...opts } : null);
  function update() {
    const st = findState(view.state);
    count.textContent = !find.value ? "" : st.matches.length ? `${st.current + 1} of ${st.matches.length}` : "no matches";
    q("fb-case").classList.toggle("on", opts.caseSensitive);
    q("fb-word").classList.toggle("on", opts.wholeWord);
  }
  function search() {
    setQuery(view, query());
    update();
  }
  function open(replace: boolean) {
    const { from, to, empty } = view.state.selection;
    const sel = empty ? "" : view.state.doc.textBetween(from, to, " ");
    if (sel && !sel.includes("\n") && sel.length < 200) find.value = sel;
    el.hidden = false;
    el.classList.toggle("replacing", replace);
    search();
    (replace && find.value ? repl : find).focus();
    (replace && find.value ? repl : find).select();
  }
  function close() {
    el.hidden = true;
    setQuery(view, null);
    view.focus();
  }
  const go = (d: number) => {
    step(view, d);
    update();
  };
  function one() {
    replaceCurrent(view, repl.value);
    update();
  }
  function all() {
    const n = replaceAll(view, repl.value, host.tracking(), host.author());
    update();
    if (n) host.toast(`Replaced ${n}${host.tracking() ? " (tracked)" : ""}. Ctrl+Z takes them back.`);
  }

  find.addEventListener("input", search);
  find.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      go(e.shiftKey ? -1 : 1);
    }
  });
  repl.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (e.ctrlKey || e.metaKey) all();
      else one();
    }
  });
  // Keys stay in the bar (they'd edit the document otherwise); Ctrl+F / Ctrl+H switch mode here.
  el.addEventListener("keydown", (e) => {
    const k = e.key.toLowerCase();
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if ((e.ctrlKey || e.metaKey) && (k === "f" || k === "h")) {
      e.preventDefault();
      open(k === "h");
    }
    e.stopPropagation();
  });
  q("fb-prev").onclick = () => go(-1);
  q("fb-next").onclick = () => go(1);
  q("fb-case").onclick = () => {
    opts.caseSensitive = !opts.caseSensitive;
    search();
  };
  q("fb-word").onclick = () => {
    opts.wholeWord = !opts.wholeWord;
    search();
  };
  q("fb-close").onclick = close;
  q("fb-one").onclick = one;
  q("fb-all").onclick = all;

  return {
    open,
    close,
    isOpen: () => !el.hidden,
    /** After the document was reloaded (the plugin state starts over). */
    refresh() {
      if (!el.hidden) search();
    },
  };
}
