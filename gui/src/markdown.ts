// Markdown shortcuts while typing, and pasting Markdown as formatted text.

import { invoke } from "@tauri-apps/api/core";
import { InputRule, inputRules, undoInputRule } from "prosemirror-inputrules";
import { Slice, type MarkType } from "prosemirror-model";
import type { Command, EditorState } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import { modelToDoc } from "./convert";
import { schema } from "./schema";
import type { DocModel } from "./types";

function inCodePara(state: EditorState, pos: number) {
  const p = state.doc.resolve(pos).parent;
  return p.type === schema.nodes.para && p.attrs.kind === "code";
}

/** `## ` and friends at the start of a paragraph set its style. */
function blockRule(re: RegExp, attrs: (m: RegExpMatchArray) => { kind: string; level: number }) {
  return new InputRule(re, (state, match, start, end) => {
    const $start = state.doc.resolve(start);
    const p = $start.parent;
    if (p.type !== schema.nodes.para || p.attrs.kind === "code") return null;
    return state.tr.delete(start, end).setNodeMarkup($start.before(), undefined, attrs(match));
  });
}

/** `**text**` and friends: drop the delimiters, apply the mark. */
function markRule(re: RegExp, mark: MarkType, attrs?: (m: RegExpMatchArray) => Record<string, unknown>) {
  return new InputRule(re, (state, match, start, end) => {
    if (inCodePara(state, start)) return null;
    const inner = match[1];
    const lead = match[0].indexOf(inner);
    const innerFrom = start + lead;
    const innerTo = innerFrom + inner.length;
    if (innerTo > end) return null;
    const tr = state.tr.delete(innerTo, end).delete(start, innerFrom);
    tr.addMark(start, start + inner.length, mark.create(attrs?.(match)));
    tr.removeStoredMark(mark);
    return tr;
  });
}

/** `$x$` and `$$x$$` become formulas (stored in Word as equations). */
function mathRule(re: RegExp, display: boolean) {
  return new InputRule(re, (state, match, start, end) => {
    if (inCodePara(state, start)) return null;
    const node = schema.nodes.math.create({ latex: match[1].trim(), display });
    return state.tr.replaceWith(start, end, node);
  });
}

const level = (spaces: string) => Math.min(8, Math.floor(spaces.length / 2));

export function markdownRuleList(): InputRule[] {
  const { marks } = schema;
  return [
      blockRule(/^(#{1,6})\s$/, (m) => ({ kind: `h${m[1].length}`, level: 0 })),
      blockRule(/^(\s*)[-*+]\s$/, (m) => ({ kind: "bullet", level: level(m[1]) })),
      blockRule(/^(\s*)\d+[.)]\s$/, (m) => ({ kind: "number", level: level(m[1]) })),
      blockRule(/^>\s$/, () => ({ kind: "quote", level: 0 })),
      blockRule(/^```$/, () => ({ kind: "code", level: 0 })),
      blockRule(/^:\s$/, () => ({ kind: "caption", level: 0 })),
      markRule(/(?<![*\w])\*\*([^*\s](?:[^*]*[^*\s])?)\*\*$/, marks.strong),
      markRule(/(?<![_\w])__([^_\s](?:[^_]*[^_\s])?)__$/, marks.strong),
      markRule(/(?<![*\w])\*([^*\s](?:[^*]*[^*\s])?)\*$/, marks.em),
      markRule(/(?<![_\w])_([^_\s](?:[^_]*[^_\s])?)_$/, marks.em),
      markRule(/(?<!~)~~([^~\s](?:[^~]*[^~\s])?)~~$/, marks.strike),
      markRule(/(?<!`)`([^`]+)`$/, marks.code),
      markRule(/\[([^\]]+)\]\(([^)\s]+)\)$/, marks.link, (m) => ({ href: m[2] })),
      mathRule(/(?<!\$)\$\$([^$]+)\$\$$/, true),
      mathRule(/(?<![$\\\w])\$([^$\s](?:[^$]*[^$\s])?)\$$/, false),
  ];
}

export function markdownRules() {
  return inputRules({ rules: markdownRuleList() });
}

/** Backspace: undo a shortcut just applied, or turn a styled paragraph back
 *  into normal text when the cursor is at its very start. */
export const markdownBackspace: Command = (state, dispatch, view) => {
  if (undoInputRule(state, dispatch, view)) return true;
  const { $cursor } = state.selection as unknown as { $cursor?: import("prosemirror-model").ResolvedPos };
  if (!$cursor || $cursor.parentOffset !== 0) return false;
  const p = $cursor.parent;
  if (p.type !== schema.nodes.para || p.attrs.kind === "normal") return false;
  const list = p.attrs.kind === "bullet" || p.attrs.kind === "number";
  const attrs = list && p.attrs.level > 0 ? { ...p.attrs, level: p.attrs.level - 1 } : { kind: "normal", level: 0 };
  if (dispatch) dispatch(state.tr.setNodeMarkup($cursor.before(), undefined, attrs));
  return true;
};

const MD_SIGNS = /^(#{1,6} |\s*[-*+] |\s*\d+[.)] |> |```|\|.*\|)|\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\([^)]+\)/m;

/** Paste Markdown (plain text that looks like it) as formatted content. */
export function pasteMarkdown(view: EditorView, e: ClipboardEvent, track: () => { on: boolean; author: string }): boolean {
  const text = e.clipboardData?.getData("text/plain") ?? "";
  const html = e.clipboardData?.getData("text/html") ?? "";
  if (!text || (html && !/<pre|<code/i.test(html)) || !MD_SIGNS.test(text) || inCodePara(view.state, view.state.selection.from)) return false;
  e.preventDefault();
  invoke<DocModel>("md_to_model", { md: text })
    .then((m) => {
      const doc = modelToDoc(m);
      const single = doc.childCount === 1 && doc.firstChild!.type === schema.nodes.para;
      const slice = single ? new Slice(doc.content, 1, 1) : new Slice(doc.content, 0, 0);
      const from = view.state.selection.from;
      const tr = view.state.tr.replaceSelection(slice).setMeta("docex-raw", true);
      const { on, author } = track();
      if (on) tr.addMark(from, tr.selection.to, schema.marks.ins.create({ author, date: "", wid: "" }));
      view.dispatch(tr.scrollIntoView());
    })
    .catch(() => {
      view.dispatch(view.state.tr.insertText(text).scrollIntoView());
    });
  return true;
}
