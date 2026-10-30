import "katex/dist/katex.min.css";
import "./style.css";
import "./tooltip";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { baseKeymap, splitBlock, toggleMark } from "prosemirror-commands";
import { dropCursor } from "prosemirror-dropcursor";
import { gapCursor } from "prosemirror-gapcursor";
import { history, redo, undo } from "prosemirror-history";
import { keymap } from "prosemirror-keymap";
import type { MarkType, Node as PMNode } from "prosemirror-model";
import { type Command, EditorState, TextSelection, type Transaction } from "prosemirror-state";
import {
  addColumnAfter,
  addColumnBefore,
  addRowAfter,
  addRowBefore,
  deleteColumn,
  deleteRow,
  goToNextCell,
  isInTable,
  mergeCells,
  splitCell,
  tableEditing,
} from "prosemirror-tables";
import { EditorView } from "prosemirror-view";
import {
  Asterisk,
  BetweenHorizontalEnd,
  BetweenHorizontalStart,
  BetweenVerticalEnd,
  BetweenVerticalStart,
  Bold,
  ChevronDown,
  Code,
  createElement,
  FilePlus,
  FolderOpen,
  Highlighter,
  Image,
  Italic,
  Link,
  MessageSquarePlus,
  PanelLeft,
  PanelRight,
  Redo2,
  Save,
  SaveAll,
  Sigma,
  Sparkles,
  Command as CommandIcon,
  Table,
  TableCellsMerge,
  TableCellsSplit,
  Trash2,
  Underline,
  Undo2,
} from "lucide";
import { initTheme } from "./theme";
import { highlightPlugin, setHighlight } from "./highlight";
import { markdownBackspace, markdownRules, pasteMarkdown } from "./markdown";
import { aiSettingsDialog, askDocument, draftReply, REWRITES, rewriteSelection, summarizeReview, type AIHost } from "./ai";
import { openPalette, type Entry } from "./palette";
import { computeSince, paraTexts, setSince, sincePlugin, type SinceResult, type Snapshot } from "./since";
import { openTriage } from "./triage";
import type { Item } from "./analyze";
import { createCockpit } from "./cockpit";
import { atomIndex, docToModel, modelToDoc, type PendingComment } from "./convert";
import { imageUrls, KINDS, renderMath, schema } from "./schema";
import { trackTransaction } from "./track";
import { setGraphics } from "./chart";
import { createFindBar, findPlugin } from "./find";
import * as reviewLog from "./review-log";
import type { Change, Comment, HeaderFooter, View } from "./types";

const $ = <T extends HTMLElement>(sel: string) => document.querySelector(sel) as T;
const h = <K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> & { class?: string } = {}, ...kids: (Node | string)[]) => {
  const el = document.createElement(tag);
  const { class: cls, ...rest } = props as { class?: string };
  if (cls) el.className = cls;
  Object.assign(el, rest);
  for (const k of kids) el.append(k);
  return el;
};

let current: View | null = null;
let tracking = false;
let author = "";
let localDirty = false;
let busy = false;
let sectionTitle = "";

// ---------------------------------------------------------------------------
// Small UI helpers

/** Toasts with a button; only the newest one stays, since it acts on the latest state. */
const dropActionToasts = () => document.querySelectorAll("#toasts .toast.has-act").forEach((t) => t.remove());

function toast(msg: string, kind: "info" | "error" = "info", action?: { label: string; run: () => unknown }) {
  const t = h("div", { class: `toast ${kind}` }, msg);
  if (action) {
    dropActionToasts();
    t.classList.add("has-act");
    const b = h("button", { class: "toast-act", textContent: action.label, onclick: () => { t.remove(); void action.run(); } });
    t.append(b);
  }
  $("#toasts").append(t);
  setTimeout(() => t.remove(), kind === "error" || action ? 8000 : 4000);
}

/** Take back the last accept / reject / resolve / delete. */
async function undoReview() {
  if (!current?.can_undo) return false;
  dropActionToasts();
  const v = await call("undo_review");
  if (v) {
    reviewLog.undoLast();
    cockpit.rerender();
    toast("Undone");
  }
  return true;
}

/** Ctrl+Z: the editor's own history first, then the last review decision. */
const undoAny: Command = (state, dispatch) => {
  if (undo(state, dispatch)) return true;
  if (!current?.can_undo || !dispatch) return !!current?.can_undo;
  void undoReview();
  return true;
};

/** A modal text prompt. Resolves to null when cancelled. */
function askText(title: string, initial = "", multiline = false): Promise<string | null> {
  return new Promise((resolve) => {
    const modal = $("#modal");
    const input = multiline ? h("textarea", { value: initial, rows: 4 }) : h("input", { value: initial });
    const done = (v: string | null) => {
      modal.hidden = true;
      modal.replaceChildren();
      view.focus();
      resolve(v);
    };
    const ok = h("button", { class: "primary", textContent: "OK", onclick: () => done(input.value) });
    const cancel = h("button", { textContent: "Cancel", onclick: () => done(null) });
    input.addEventListener("keydown", (e) => {
      const ke = e as KeyboardEvent;
      if (ke.key === "Escape") done(null);
      if (ke.key === "Enter" && (!multiline || ke.ctrlKey || ke.metaKey)) {
        e.preventDefault();
        done(input.value);
      }
    });
    modal.replaceChildren(h("div", { class: "dialog" }, h("label", {}, title), input, h("div", { class: "row" }, cancel, ok)));
    modal.hidden = false;
    input.focus();
    (input as HTMLInputElement).select?.();
  });
}

/** Formula editor with a live preview. Resolves to null when cancelled. */
function editMath(latex: string, display: boolean): Promise<{ latex: string; display: boolean } | null> {
  return new Promise((resolve) => {
    const modal = $("#modal");
    const input = h("textarea", { value: latex, rows: 3, className: "latex", spellcheck: false });
    const preview = h("div", { class: "math-preview" });
    const disp = h("input", { type: "checkbox", checked: display }) as HTMLInputElement;
    const update = () => renderMath(preview, input.value, disp.checked);
    const done = (ok: boolean) => {
      modal.hidden = true;
      modal.replaceChildren();
      view.focus();
      resolve(ok && input.value.trim() ? { latex: input.value.trim(), display: disp.checked } : null);
    };
    input.addEventListener("input", update);
    disp.addEventListener("change", update);
    input.addEventListener("keydown", (e) => {
      const ke = e as KeyboardEvent;
      if (ke.key === "Escape") done(false);
      if (ke.key === "Enter" && (ke.ctrlKey || ke.metaKey || !ke.shiftKey)) {
        e.preventDefault();
        done(true);
      }
    });
    modal.replaceChildren(
      h(
        "div",
        { class: "dialog" },
        h("label", {}, "Formula (LaTeX)"),
        input,
        preview,
        h(
          "div",
          { class: "row" },
          h("label", { class: "check" }, disp, " own line (display)"),
          h("span", { class: "grow" }),
          h("button", { textContent: "Cancel", onclick: () => done(false) }),
          h("button", { class: "primary", textContent: "OK", onclick: () => done(true) }),
        ),
        h("div", { class: "hint" }, "Saved as a Word equation. Enter: OK · Shift+Enter: new line · Esc: cancel"),
      ),
    );
    modal.hidden = false;
    update();
    input.focus();
  });
}

async function insertMath(pos?: number) {
  const node = pos !== undefined ? view.state.doc.nodeAt(pos) : null;
  const r = await editMath(node?.attrs.latex ?? "", node?.attrs.display ?? false);
  if (!r) return;
  const m = schema.nodes.math.create(r);
  const tr = pos !== undefined && node ? view.state.tr.replaceWith(pos, pos + node.nodeSize, m) : view.state.tr.replaceSelectionWith(m);
  view.dispatch(tr.scrollIntoView());
}

function shortDate(d: string) {
  return d ? d.replace("T", " ").replace(/:\d\dZ$/, "") : "";
}

function basename(p: string) {
  return p.split(/[\\/]/).pop() || p;
}

// ---------------------------------------------------------------------------
// Editor

const setKind = (kind: string): Command => (state, dispatch) => {
  const tr = state.tr;
  state.doc.nodesBetween(state.selection.from, state.selection.to, (n, pos) => {
    if (n.type === schema.nodes.para) {
      const keepLevel = kind === "bullet" || kind === "number";
      tr.setNodeMarkup(pos, undefined, { kind, level: keepLevel ? n.attrs.level : 0 });
      return false;
    }
    return true;
  });
  if (dispatch) dispatch(tr);
  return true;
};

const shiftLevel = (d: number): Command => (state, dispatch) => {
  const tr = state.tr;
  let any = false;
  state.doc.nodesBetween(state.selection.from, state.selection.to, (n, pos) => {
    if (n.type === schema.nodes.para && (n.attrs.kind === "bullet" || n.attrs.kind === "number")) {
      tr.setNodeMarkup(pos, undefined, { ...n.attrs, level: Math.max(0, Math.min(8, n.attrs.level + d)) });
      any = true;
      return false;
    }
    return true;
  });
  if (any && dispatch) dispatch(tr);
  return any;
};

/** Enter: lists, quotes and code continue; an empty list item ends the list. */
const splitKeep: Command = (state, dispatch) => {
  const { $from } = state.selection;
  const p = $from.parent;
  if (p.type !== schema.nodes.para) return false;
  const { kind, level } = p.attrs;
  const list = kind === "bullet" || kind === "number";
  if (list && p.content.size === 0) {
    if (dispatch) dispatch(state.tr.setNodeMarkup($from.before(), undefined, { kind: "normal", level: 0 }));
    return true;
  }
  const atEnd = $from.parentOffset === p.content.size;
  const keep = list || kind === "quote" || kind === "code" || !atEnd;
  return splitBlock(state, (tr) => {
    if (!dispatch) return;
    const $pos = tr.selection.$from;
    if ($pos.parent.type === schema.nodes.para) {
      tr.setNodeMarkup($pos.before(), undefined, keep ? { kind, level } : { kind: "normal", level: 0 });
    }
    dispatch(tr);
  });
};

const toggleLink: Command = (state, dispatch) => {
  const { from, to, empty } = state.selection;
  if (empty) return false;
  if (state.doc.rangeHasMark(from, to, schema.marks.link)) {
    if (dispatch) dispatch(state.tr.removeMark(from, to, schema.marks.link));
    return true;
  }
  if (dispatch) {
    askText("Link address").then((href) => {
      if (href) view.dispatch(view.state.tr.addMark(from, to, schema.marks.link.create({ href })));
    });
  }
  return true;
};

function plugins() {
  return [
    history(),
    markdownRules(),
    keymap({
      "Mod-z": undoAny,
      "Mod-y": redo,
      "Shift-Mod-z": redo,
      "Mod-b": toggleMark(schema.marks.strong),
      "Mod-i": toggleMark(schema.marks.em),
      "Mod-`": toggleMark(schema.marks.code),
      "Mod-u": toggleMark(schema.marks.underline),
      // Word's keys for raised / lowered text.
      "Shift-Mod-=": toggleMark(schema.marks.sup),
      "Shift-Mod-+": toggleMark(schema.marks.sup),
      "Mod-=": toggleMark(schema.marks.sub),
      "Mod-k": toggleLink,
      Enter: splitKeep,
      "Shift-Enter": (state, dispatch) => {
        if (dispatch) dispatch(state.tr.replaceSelectionWith(schema.nodes.hard_break.create()).scrollIntoView());
        return true;
      },
      Backspace: markdownBackspace,
      Tab: (s, d) => (isInTable(s) ? goToNextCell(1)(s, d) : shiftLevel(1)(s, d) || true),
      "Shift-Tab": (s, d) => (isInTable(s) ? goToNextCell(-1)(s, d) : shiftLevel(-1)(s, d) || true),
    }),
    keymap(baseKeymap),
    dropCursor(),
    gapCursor(),
    tableEditing(),
    highlightPlugin,
    findPlugin,
    sincePlugin,
  ];
}

const view = new EditorView($("#editor"), {
  attributes: () => ({ spellcheck: spellOn() ? "true" : "false" }),
  state: EditorState.create({ doc: schema.nodes.doc.create(null, schema.nodes.para.create()), plugins: plugins() }),
  dispatchTransaction(tr: Transaction) {
    let t = tr;
    if (tracking) {
      // Never lose a keystroke: if marking it as a tracked change fails,
      // apply the plain edit (the core still records it on save).
      try {
        t = trackTransaction(view.state, tr, author) ?? tr;
      } catch (err) {
        console.error("docex: live tracking failed; applying the edit as is", err);
        t = tr;
      }
    }
    view.updateState(view.state.apply(t));
    if (t.docChanged) {
      cockpit.refreshSoon();
      keepRecoverySoon();
      if (!localDirty) {
        localDirty = true;
        renderStatus();
      }
    }
    updateToolbar();
  },
  handleClickOn(_v, _pos, node, nodePos) {
    if (node.type === schema.nodes.math) {
      void insertMath(nodePos);
      return true;
    }
    return false;
  },
  handlePaste(v, e) {
    return pasteMarkdown(v, e, () => ({ on: tracking, author }));
  },
  handleDOMEvents: {
    click(_v, e) {
      const el = (e.target as HTMLElement).closest("ins.chg, del.chg") as HTMLElement | null;
      if (el && el.dataset.wid) {
        showChangePopover(el);
        cockpit.focusWid(el.dataset.wid);
      } else hidePopover();
      const c = (e.target as HTMLElement).closest(".cmt") as HTMLElement | null;
      if (c) cockpit.focusComment(c.dataset.ids?.split(" ")[0] || "");
      return false;
    },
  },
});

// "Since you last saw it": the snapshot from the previous visit, for this session.
let sinceBase: Snapshot | null = null;
let sinceRes: SinceResult | null = null;
let sinceOn = false;
let sincePath: string | null = null;

function ago(t: string) {
  const s = (Date.now() - Date.parse(t)) / 1000;
  if (!Number.isFinite(s)) return "";
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} days ago`;
}

function toggleSince(on = !sinceOn) {
  if (!sinceRes) return toast("No earlier version of this document to compare with yet. docex remembers it from now on.");
  sinceOn = on;
  setSince(view, sinceRes.decos, sinceOn);
  cockpit.rerender();
  if (sinceOn && sinceRes.spots.length) {
    const c = view.coordsAtPos(sinceRes.spots[0]);
    const wrap = $("#page-wrap");
    wrap.scrollTo({ top: c.top - wrap.getBoundingClientRect().top + wrap.scrollTop - wrap.clientHeight / 3 });
  }
}

function sinceChip(): HTMLElement | null {
  if (!sinceBase || !sinceRes) return null;
  const { changed, added, removed } = sinceRes;
  const other = current?.path && sinceBase.path !== current.path && sinceBase.name !== basename(current.path) ? ` (${sinceBase.name})` : "";
  const n = changed + added + removed;
  const b = h("button", { class: `tag since${sinceOn ? " on" : ""}`, onclick: () => toggleSince() }, n ? `since ${ago(sinceBase.taken)}${other}: ${[changed && `${changed} changed`, added && `${added} new`, removed && `${removed} removed`].filter(Boolean).join(", ")}` : `unchanged since ${ago(sinceBase.taken)}${other}`);
  b.dataset.tip = "What differs from the version you last opened or saved, tracked or not (click: highlight)";
  return b;
}

const aiHost: AIHost = { view, author: () => author, toast, index: () => cockpit.index() };

const sectionOf = (it: Item) => cockpit.index().sections[it.section]?.title ?? "";

/** "Done reviewing": what you decided, as text to paste into a reply. */
function showSummary() {
  const idx = cockpit.index();
  const text = reviewLog.summaryText({
    file: current?.path ? basename(current.path) : "Untitled",
    me: author,
    open: idx.items.filter((i) => i.kind === "comment" && !i.comment?.done),
    remaining: idx.items.filter((i) => i.kind !== "comment" && !i.pending && i.author !== author).length,
    mine: idx.items.filter((i) => i.kind !== "comment" && i.author === author).length,
  });
  const modal = $("#modal");
  const area = h("textarea", { value: text, rows: 16, className: "summary-text", readOnly: true });
  const close = () => {
    modal.hidden = true;
    modal.replaceChildren();
    view.focus();
  };
  const copy = h("button", {
    class: "primary",
    textContent: "Copy",
    onclick: () => {
      area.select();
      document.execCommand("copy");
      toast("Summary copied");
      close();
    },
  });
  area.addEventListener("keydown", (e) => (e as KeyboardEvent).key === "Escape" && close());
  modal.replaceChildren(h("div", { class: "dialog wide" }, h("label", {}, "Review summary"), area, h("div", { class: "row" }, h("button", { textContent: "Close", onclick: close }), copy)));
  modal.hidden = false;
  area.focus();
}

/** A comment on the range of a change (used from triage). */
async function commentOn(it: Item, text: string) {
  if (it.from === null) return toast("This change has no place in the text to comment on.");
  const $f = view.state.doc.resolve(it.from);
  const p = $f.parent;
  const end = Math.min(it.to ?? it.from, $f.end());
  await call("sync", { pending: { pos: $f.before(), start: atomIndex(p, $f.parentOffset), end: atomIndex(p, end - $f.start()), text } });
}

const cockpit = createCockpit({
  view,
  current: () => current,
  me: () => author,
  call: (cmd, args) => call(cmd, args),
  toast,
  tracking: () => tracking,
  triage: (items, scope) =>
    openTriage(items, scope, {
      show: (it, inset) => cockpit.focus(it, true, { hold: true, inset }),
      apply: async (d, label) => {
        const v = await call("resolve_batch", {
          accept: d.accept.flatMap((i) => i.wids),
          reject: d.reject.flatMap((i) => i.wids),
          done_comments: d.resolve.map((i) => i.comment!.id),
          label,
        });
        const others = (l: Item[]) => l.filter((i) => i.author !== author);
        if (v) {
          reviewLog.recordMany([["accepted", others(d.accept)], ["rejected", others(d.reject)], ["resolved", d.resolve]], sectionOf);
          cockpit.rerender();
        }
      },
      reply: async (it, text) => {
        if (!it.comment) return commentOn(it, text);
        if (await call("comment_reply", { id: it.comment.id, text })) {
          reviewLog.record("replied", [it], sectionOf);
          cockpit.rerender();
        }
      },
      ask: (title) => askText(title, "", true),
      section: (it) => cockpit.index().sections[it.section]?.title ?? "",
      done: () => {
        setHighlight(view, null);
        view.focus();
        cockpit.refreshNow();
      },
    }),
  summarize: (items, scope) => void summarizeReview(aiHost, items, scope),
  summary: () => showSummary(),
  draft: (it, onText) => draftReply(aiHost, it, onText),
  sinceChip,
});
cockpit.onSectionChange((t) => {
  sectionTitle = t;
  renderStatus();
});

const findBar = createFindBar(view, { tracking: () => tracking, author: () => author, toast });

// Headers and footers: faint, read-only strips above and below the text.
const pageHeader = h("div", { class: "furniture top" });
const pageFooter = h("div", { class: "furniture bottom" });
view.dom.before(pageHeader);
view.dom.after(pageFooter);

function renderFurniture(list: HeaderFooter[]) {
  const fill = (el: HTMLElement, footer: boolean) => {
    el.replaceChildren(
      ...list
        .filter((f) => f.footer === footer)
        .map((f) => {
          const tag = f.which === "first" ? "first page" : f.which === "even" ? "even pages" : "";
          const row = h("div", { class: "furn" }, ...f.lines.map((l) => h("div", {}, l)));
          if (tag) row.dataset.which = tag;
          return row;
        }),
    );
    el.title = footer ? "Footer (edit in Word)" : "Header (edit in Word)";
  };
  fill(pageHeader, false);
  fill(pageFooter, true);
}

function load(v: View, keepSelection = true) {
  current = v;
  tracking = v.track;
  for (const [k, url] of Object.entries(v.images)) imageUrls.set(k, url);
  setGraphics(v.model.graphics);
  reviewLog.resetFor(v.path);
  if (v.lang !== spellLang) {
    spellLang = v.lang ?? null;
    void applySpellcheck();
  }
  if (v.dirty) keepRecoverySoon();
  const sel = view.state.selection;
  const doc = modelToDoc(v.model);
  let state = EditorState.create({ doc, plugins: plugins() });
  if (keepSelection) {
    const pos = Math.min(sel.from, doc.content.size);
    try {
      state = state.apply(state.tr.setSelection(TextSelection.near(doc.resolve(pos))));
    } catch {
      /* keep the default selection */
    }
  }
  view.updateState(state);
  findBar.refresh();
  renderFurniture(v.model.furniture ?? []);
  localDirty = false;
  if (!v.can_undo) dropActionToasts();
  for (const w of v.warnings) toast(w);
  // Compare with the previous visit (fetched once per opened file).
  if (v.path && v.path !== sincePath) {
    sincePath = v.path;
    sinceBase = null;
    sinceRes = null;
    sinceOn = false;
    const path = v.path;
    invoke<Snapshot | null>("snapshot_swap", { path, paras: paraTexts(view.state.doc) })
      .then((prev) => {
        if (path !== sincePath || !prev) return;
        sinceBase = prev;
        sinceRes = computeSince(view.state.doc, prev);
        setSince(view, sinceRes.decos, sinceOn);
        cockpit.rerender();
      })
      .catch(() => {});
  } else if (sinceBase) {
    sinceRes = computeSince(view.state.doc, sinceBase);
    setSince(view, sinceRes.decos, sinceOn);
  }
  cockpit.refreshNow();
  renderStatus();
  updateToolbar();
  void getCurrentWindow().setTitle(`${v.path ? basename(v.path) : "Untitled"}${v.dirty ? " •" : ""} — docex`).catch(() => {});
}

function model(pending?: PendingComment) {
  return docToModel(view.state.doc, pending);
}

function isDirty() {
  return localDirty || !!current?.dirty;
}

/** Call a backend command with the current editor content; reload the result. */
async function call(cmd: string, args: Record<string, unknown> = {}, withModel = true) {
  if (busy) return;
  busy = true;
  document.body.classList.add("busy");
  try {
    const payload = withModel ? { model: model(args.pending as PendingComment | undefined), tracked: tracking, ...args } : { ...args };
    const label = args.label as string | undefined;
    delete (payload as Record<string, unknown>).pending;
    delete (payload as Record<string, unknown>).label;
    const v = await invoke<View>(cmd, payload);
    load(v);
    if (label && v.can_undo) toast(label, "info", { label: "Undo", run: undoReview });
    return v;
  } catch (e) {
    toast(String(e), "error");
  } finally {
    busy = false;
    document.body.classList.remove("busy");
  }
}

// ---------------------------------------------------------------------------
// Commands

async function confirmDiscard() {
  if (!isDirty()) return true;
  const yes = await ask("Discard unsaved changes?", { title: "docex", kind: "warning" });
  if (yes) dropRecovery();
  return yes;
}

// Spell checking: the document's language and the system's, as far as
// dictionaries are installed (WebKitGTK uses Enchant / Hunspell).
let spellLang: string | null | undefined;
let spellWarned = "";
function spellOn() {
  return localStorage.getItem("docex-spell") !== "off";
}

async function applySpellcheck() {
  const want = [spellLang, navigator.language].filter((l): l is string => !!l).map((l) => l.replace("-", "_"));
  const used = spellOn() ? await invoke<string[]>("spellcheck", { langs: want }).catch(() => null) : await invoke<string[]>("spellcheck", { langs: [] }).catch(() => null);
  view.setProps({});
  if (!spellOn() || !used) return;
  const missing = [...new Set(want.map((l) => l.split("_")[0]))].filter((b) => !used.some((u) => u.split("_")[0] === b));
  const key = missing.join(",");
  if (missing.length && key !== spellWarned) {
    spellWarned = key;
    const name = (b: string) => new Intl.DisplayNames(["en"], { type: "language" }).of(b) ?? b;
    toast(`No spelling dictionary for ${missing.map(name).join(" or ")}: install one (Arch: hunspell-${missing[0] === "en" ? "en_us" : missing[0]}) and reopen the document.`);
  }
}

// Crash recovery: while there are unsaved changes, the backend keeps a copy of
// the document with them a few seconds after the last edit. Saving or
// discarding removes it; opening the file again offers it.
let recoveryTimer: number | undefined;

function keepRecoverySoon() {
  clearTimeout(recoveryTimer);
  recoveryTimer = window.setTimeout(() => {
    if (current?.path && isDirty()) void invoke("recovery_put", { model: model(), tracked: tracking }).catch(() => {});
  }, 4000);
}

function dropRecovery() {
  clearTimeout(recoveryTimer);
  if (current?.path) void invoke("recovery_clear", { path: current.path }).catch(() => {});
}

/** After opening `path`: restore unsaved edits from a crash, if the user wants them. */
async function offerRecovery(path: string) {
  const saved = await invoke<string | null>("recovery_check", { path }).catch(() => null);
  if (!saved) return;
  const when = new Date(saved).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
  const restore = await ask(`docex closed with unsaved changes to ${basename(path)} (${when}). Restore them?\n\nThe file itself is unchanged until you save.`, { title: "docex", kind: "warning", okLabel: "Restore", cancelLabel: "Discard" });
  if (restore) {
    await call("recovery_open", { path }, false);
    toast("Unsaved changes restored. Save to keep them.");
  } else void invoke("recovery_clear", { path }).catch(() => {});
}

async function openFile(path?: string) {
  if (!path) {
    if (!(await confirmDiscard())) return;
    const p = await openDialog({ multiple: false, filters: [{ name: "Word document", extensions: ["docx"] }] });
    if (!p) return;
    path = p as string;
  }
  const v = await call("open_doc", { path }, false);
  if (v?.path) {
    const recent = [v.path, ...recentFiles().filter((r) => r !== v.path)].slice(0, 12);
    localStorage.setItem("docex-recent", JSON.stringify(recent));
    await offerRecovery(v.path);
  }
  view.focus();
}

async function newFile() {
  if (!(await confirmDiscard())) return;
  await call("new_doc", {}, false);
  view.focus();
}

async function saveFile(as = false) {
  let path: string | null = null;
  if (as || !current?.path) {
    path = await saveDialog({ filters: [{ name: "Word document", extensions: ["docx"] }], defaultPath: current?.path ?? "Untitled.docx" });
    if (!path) return;
    if (!path.toLowerCase().endsWith(".docx")) path += ".docx";
  }
  const v = await call("save", { path });
  if (v) {
    toast(`Saved ${basename(v.path || "")}`);
    if (v.path) void invoke("snapshot_put", { path: v.path, paras: paraTexts(view.state.doc) }).catch(() => {});
  }
}

async function toggleTrack() {
  await call("set_track", { on: !tracking });
  toast(`Track changes ${tracking ? "on" : "off"}`);
}

async function insertImage() {
  const p = await openDialog({ multiple: false, filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "gif"] }] });
  if (!p) return;
  const path = p as string;
  try {
    imageUrls.set(path, await invoke<string>("image_data", { path }));
  } catch (e) {
    toast(String(e), "error");
    return;
  }
  view.dispatch(view.state.tr.replaceSelectionWith(schema.nodes.image.create({ src: path, alt: "" })).scrollIntoView());
  view.focus();
}

function insertTable() {
  const { table, table_row, table_cell, para } = schema.nodes;
  const t = table.create(null, [0, 1, 2].map(() => table_row.create(null, [0, 1, 2].map(() => table_cell.create(null, para.create())))));
  const tr = view.state.tr.replaceSelectionWith(t);
  view.dispatch(tr.scrollIntoView());
  view.focus();
}

function insertFootnote() {
  const label = `new${Date.now().toString(36)}`;
  let tr = view.state.tr.replaceSelectionWith(schema.nodes.fnref.create({ label }));
  tr = tr.insert(tr.doc.content.size, schema.nodes.footnote.create({ label }, schema.nodes.para.create()));
  tr.setSelection(TextSelection.create(tr.doc, tr.doc.content.size - 2));
  view.dispatch(tr.scrollIntoView());
  view.focus();
}

async function addComment() {
  const { $from, $to, empty } = view.state.selection;
  if (empty || !$from.sameParent($to) || $from.parent.type !== schema.nodes.para) {
    toast("Select some text within one paragraph to comment on it.");
    return;
  }
  const p: PMNode = $from.parent;
  const sel = { pos: $from.before(), start: atomIndex(p, $from.parentOffset), end: atomIndex(p, $to.parentOffset) };
  const text = await askText("Comment", "", true);
  if (!text?.trim()) return;
  await call("sync", { pending: { ...sel, text: text.trim() } });
}

const accept = (wids: string[], yes: boolean) => call("resolve", { wids, accept: yes, label: yes ? "Accepted the change" : "Rejected the change" });

// ---------------------------------------------------------------------------
// Popover for tracked changes in the text

function hidePopover() {
  $("#popover").hidden = true;
}

function showChangePopover(el: HTMLElement) {
  const pop = $("#popover");
  const wid = el.dataset.wid!;
  // The review item covers both halves of a replacement, like the list does.
  const it = cockpit.itemByWid(wid);
  const ch = current?.changes.find((c) => c.wids.includes(wid));
  const decide = (yes: boolean) => {
    hidePopover();
    void (it ? cockpit.resolve([it], yes) : accept(ch ? ch.wids : [wid], yes));
  };
  const btn = (label: string, cls: string, yes: boolean) => {
    const b = h("button", { class: cls, textContent: label, onclick: () => decide(yes) });
    b.dataset.tip = `${label}  Alt+${label[0]}`;
    return b;
  };
  const note = h("button", {
    textContent: "Comment…",
    onclick: async () => {
      hidePopover();
      const text = await askText("Comment on this change", "", true);
      if (!text?.trim()) return;
      if (it) await commentOn(it, text.trim());
      else toast("Save first to comment on this change.");
    },
  });
  note.dataset.tip = "Add a comment on this change";
  pop.replaceChildren(
    h("div", { class: "pop-title" }, it ? `${it.author} · ${it.label ?? ({ insert: "inserted", delete: "deleted", replace: "replaced" } as Record<string, string>)[it.kind] ?? "changed"} · ${shortDate(it.date)}` : el.title),
    it && it.kind === "replace" ? cockpit.changeLine({ ...it, before: "", after: "" }) : "",
    h("div", { class: "row" }, btn("Accept", "ok", true), btn("Reject", "no", false), note),
  );
  const r = el.getBoundingClientRect();
  pop.style.left = "0px";
  pop.hidden = false;
  pop.style.left = `${Math.max(8, Math.min(r.left, window.innerWidth - pop.offsetWidth - 8))}px`;
  pop.style.top = `${r.bottom + 6}px`;
}

function renderStatus() {
  const s = $("#status");
  const dirty = isDirty();
  s.replaceChildren(
    h("span", { class: "path", title: current?.path ?? "" }, current?.path ?? "Untitled"),
    dirty ? h("span", { class: "dirty" }, "● unsaved") : "",
    sectionTitle ? h("span", { class: "section" }, `§ ${sectionTitle}`) : "",
    h("span", { class: "spacer" }),
    h("span", {}, tracking ? "tracking changes" : "editing directly"),
    h("button", {
      class: "author",
      title: "",
      onclick: async () => {
        const a = await askText("Your name (for changes and comments)", author);
        if (a?.trim()) {
          author = a.trim();
          localStorage.setItem("docex-author", author);
          await invoke("set_author", { name: author });
          renderStatus();
        }
      },
    }, `@${author}`),
  );
  s.querySelector<HTMLElement>(".author")!.dataset.tip = "Your name on changes and comments  click to change";
  if (typeof updateToolbar === "function") updateToolbar();
}

// ---------------------------------------------------------------------------
// Toolbar

type IconNode = Parameters<typeof createElement>[0];

function icon(node: IconNode): SVGElement {
  const el = createElement(node);
  el.setAttribute("class", "ico");
  return el;
}

/** An icon button; `tip` shows on hover. */
function tb(ic: IconNode, tip: string, run: () => unknown, label?: string): HTMLButtonElement {
  const b = h("button", { class: "tb", onmousedown: (e: MouseEvent) => e.preventDefault(), onclick: () => void run() });
  b.dataset.tip = tip;
  b.append(icon(ic));
  if (label) b.append(h("span", { class: "lbl" }, label));
  return b;
}

const sep = () => h("span", { class: "sep" });

const kindSelect = h("select");
kindSelect.dataset.tip = "Paragraph style  or type # , - , 1. , > at the start of a line";
for (const [k, label] of KINDS) kindSelect.append(h("option", { value: k }, label));
kindSelect.onchange = () => {
  setKind(kindSelect.value)(view.state, view.dispatch);
  view.focus();
};
const styles = h("span", { class: "styles" }, kindSelect, icon(ChevronDown));

const markButtons: [HTMLButtonElement, MarkType][] = [
  [tb(Bold, "Bold  Ctrl+B", () => toggleMark(schema.marks.strong)(view.state, view.dispatch)), schema.marks.strong],
  [tb(Italic, "Italic  Ctrl+I", () => toggleMark(schema.marks.em)(view.state, view.dispatch)), schema.marks.em],
  [tb(Code, "Code  Ctrl+`", () => toggleMark(schema.marks.code)(view.state, view.dispatch)), schema.marks.code],
  [tb(Underline, "Underline  Ctrl+U", () => toggleMark(schema.marks.underline)(view.state, view.dispatch)), schema.marks.underline],
  [tb(Highlighter, "Highlight (yellow)  click again to remove", () => toggleMark(schema.marks.highlight, { color: "yellow" })(view.state, view.dispatch)), schema.marks.highlight],
  [tb(Link, "Link  Ctrl+K", () => toggleLink(view.state, view.dispatch)), schema.marks.link],
];

const saveBtn = tb(Save, "Save  Ctrl+S", () => saveFile(false));
/** Table commands, shown while the cursor is in a table; each is greyed out where it can't apply. */
const TABLE_CMDS: [IconNode, string, Command, string?][] = [
  [BetweenHorizontalStart, "Insert a row above", addRowBefore],
  [BetweenHorizontalEnd, "Insert a row below", addRowAfter],
  [BetweenVerticalStart, "Insert a column on the left", addColumnBefore],
  [BetweenVerticalEnd, "Insert a column on the right", addColumnAfter],
  [TableCellsMerge, "Merge the selected cells  (drag across cells to select them)", mergeCells],
  [TableCellsSplit, "Split a merged cell", splitCell],
  [Trash2, "Delete this row", deleteRow, "row"],
  [Trash2, "Delete this column", deleteColumn, "col"],
];
const tableButtons = TABLE_CMDS.map(([ic, tip, cmd, label]) => [tb(ic, tip, () => cmd(view.state, view.dispatch), label), cmd] as const);
const tableCtx = h("span", { class: "tb-ctx" }, sep(), ...tableButtons.map(([b]) => b));

const trackSwitch = h("button", { class: "track-switch", onclick: () => void toggleTrack() }, h("span", { class: "knob" }), h("span", { class: "lbl" }, "Track changes"));
trackSwitch.dataset.tip = "Track Changes: record your edits as suggestions others can accept or reject\n(Word's setting, saved in the document)";

$("#toolbar").append(
  h("span", { class: "brand" }, "docex"),
  tb(FilePlus, "New document  Ctrl+N", newFile),
  tb(FolderOpen, "Open a .docx  Ctrl+O", () => openFile()),
  saveBtn,
  tb(SaveAll, "Save as  Ctrl+Shift+S", () => saveFile(true)),
  sep(),
  tb(Undo2, "Undo  Ctrl+Z  (also takes back the last accept, reject or resolve)", () => undoAny(view.state, view.dispatch)),
  tb(Redo2, "Redo  Ctrl+Y", () => redo(view.state, view.dispatch)),
  sep(),
  styles,
  sep(),
  ...markButtons.map(([b]) => b),
  sep(),
  tb(Image, "Insert an image from a file", insertImage),
  tb(Table, "Insert a 3×3 table", insertTable),
  tb(Asterisk, "Insert a footnote at the cursor", insertFootnote),
  tb(Sigma, "Insert a formula  or type $…$ / $$…$$", () => insertMath()),
  tb(MessageSquarePlus, "Comment on the selected text", addComment),
  tableCtx,
  sep(),
  tb(Sparkles, "AI: rewrite, ask, summarize  Ctrl+J", openAIMenu),
  tb(CommandIcon, "Command palette  Ctrl+P", () => openPalette(paletteEntries())),
  h("span", { class: "grow" }),
  trackSwitch,
  sep(),
  panelToggle("outline", PanelLeft, "Show or hide the outline  Ctrl+1"),
  panelToggle("sidebar", PanelRight, "Show or hide changes and comments  Ctrl+2"),
);

function panelToggle(id: string, ic: IconNode, tip: string) {
  const b = tb(ic, tip, () => togglePanel(id));
  b.dataset.panel = id;
  return b;
}

type PanelId = "outline" | "sidebar";
const PANELS: PanelId[] = ["outline", "sidebar"];
/** Below these window widths a panel only shows as an overlay, so the page keeps room. */
const MIN_WIDTH: Record<PanelId, number> = { outline: 1180, sidebar: 860 };
const panelPref: Record<PanelId, boolean> = { outline: true, sidebar: true };
const overlay: Record<PanelId, boolean> = { outline: false, sidebar: false };

function layout() {
  for (const id of PANELS) {
    const el = document.getElementById(id)!;
    const narrow = window.innerWidth < MIN_WIDTH[id];
    const asOverlay = narrow && overlay[id];
    el.hidden = narrow ? !asOverlay : !panelPref[id];
    el.classList.toggle("overlay", asOverlay);
    document.querySelector(`[data-panel="${id}"]`)?.classList.toggle("active", !el.hidden);
  }
}

function togglePanel(id: string) {
  const p = id as PanelId;
  if (window.innerWidth < MIN_WIDTH[p]) {
    overlay[p] = !overlay[p];
  } else {
    panelPref[p] = !panelPref[p];
    localStorage.setItem(`docex-hide-${p}`, panelPref[p] ? "" : "1");
  }
  layout();
  cockpit.refreshSoon();
}

window.addEventListener("resize", () => {
  layout();
  cockpit.refreshSoon();
});
// Clicking into the page closes overlay panels.
document.getElementById("page-area")!.addEventListener("mousedown", () => {
  if (overlay.outline || overlay.sidebar) {
    overlay.outline = overlay.sidebar = false;
    layout();
  }
});

function updateToolbar() {
  const { $from, from, to, empty } = view.state.selection;
  const p = $from.parent;
  kindSelect.value = p.type === schema.nodes.para ? p.attrs.kind : "normal";
  kindSelect.disabled = p.type !== schema.nodes.para;
  for (const [b, m] of markButtons) {
    const active = empty ? !!m.isInSet(view.state.storedMarks || $from.marks()) : view.state.doc.rangeHasMark(from, to, m);
    b.classList.toggle("active", active);
  }
  const inTable = isInTable(view.state);
  tableCtx.classList.toggle("show", inTable);
  if (inTable) for (const [b, cmd] of tableButtons) b.disabled = !cmd(view.state);
  trackSwitch.classList.toggle("on", tracking);
  saveBtn.querySelector(".dot")?.remove();
  if (isDirty()) saveBtn.append(h("span", { class: "dot" }));
}

// ---------------------------------------------------------------------------
// Command palette and AI menu

function recentFiles(): string[] {
  try {
    return JSON.parse(localStorage.getItem("docex-recent") || "[]");
  } catch {
    return [];
  }
}

function aiEntries(): Entry[] {
  const hasSel = !view.state.selection.empty;
  const target = hasSel ? "selection" : "paragraph";
  return [
    ...REWRITES.map(([label, instr]) => ({ label: `${label} (${target})`, group: "AI", hint: "tracked", run: () => rewriteSelection(aiHost, instr, label) })),
    {
      label: `Rewrite ${target} with your own instruction…`,
      group: "AI",
      run: async () => {
        const instr = await askText(`How should the ${target} change?`);
        if (instr?.trim()) await rewriteSelection(aiHost, instr.trim(), instr.trim());
      },
    },
    {
      label: "Ask about this document…",
      group: "AI",
      run: async () => {
        const q = await askText("Ask about this document");
        if (q?.trim()) await askDocument(aiHost, q.trim());
      },
    },
    { label: "Summarize the changes and comments shown", group: "AI", run: () => summarizeReview(aiHost, cockpit.shown(), "the review list") },
    { label: "Summarize all changes and comments", group: "AI", run: () => summarizeReview(aiHost, cockpit.index().items, "all changes and comments") },
    { label: "AI settings (provider, key, model)…", group: "AI", run: () => aiSettingsDialog() },
  ];
}

function openAIMenu() {
  openPalette(aiEntries(), "AI: rewrite, ask, summarize…  (rewrites become tracked changes)");
}

/** Character formatting for the palette: raised/lowered text, highlighter colours, font colours. */
function formatEntries(): Entry[] {
  const run = (cmd: Command) => () => {
    cmd(view.state, view.dispatch);
    view.focus();
  };
  const setMark = (type: MarkType, attrs: Record<string, string> | null): Command => (state, dispatch) => {
    const { from, to, empty } = state.selection;
    if (empty) return false;
    let tr = state.tr.removeMark(from, to, type);
    if (attrs) tr = tr.addMark(from, to, type.create(attrs));
    dispatch?.(tr);
    return true;
  };
  const { sup, sub, highlight, color } = schema.marks;
  return [
    { label: "Superscript", group: "Format", hint: "Ctrl+Shift+=", run: run(toggleMark(sup)) },
    { label: "Subscript", group: "Format", hint: "Ctrl+=", run: run(toggleMark(sub)) },
    ...["yellow", "green", "cyan", "magenta", "red", "blue"].map((c) => ({ label: `Highlight ${c}`, group: "Format", run: run(setMark(highlight, { color: c })) })),
    { label: "Remove highlight", group: "Format", run: run(setMark(highlight, null)) },
    ...([["red", "#C00000"], ["blue", "#0070C0"], ["green", "#00B050"], ["orange", "#ED7D31"]] as const).map(([n, c]) => ({ label: `Text colour ${n}`, group: "Format", run: run(setMark(color, { color: c })) })),
    { label: "Remove text colour", group: "Format", run: run(setMark(color, null)) },
  ];
}

function paletteEntries(): Entry[] {
  const idx = cockpit.index();
  const e: Entry[] = [
    { label: "Triage what's shown in the review list", group: "Review", hint: "Alt+T", run: () => cockpit.startTriage() },
    { label: "Triage everything", group: "Review", run: () => { cockpit.clearFilters(); return cockpit.startTriage(); } },
    { label: "Triage open comment threads", group: "Review", run: () => { cockpit.clearFilters(); cockpit.filterTo({ show: "open" }); return cockpit.startTriage(); } },
    { label: "Hide formatting / whitespace / punctuation noise", group: "Review", run: () => cockpit.filterTo({ hideNoise: true }) },
    { label: "Accept all noise (formatting, whitespace, punctuation)", group: "Review", run: () => cockpit.resolve(idx.items.filter((i) => i.noise), true) },
    { label: "Show only open comment threads", group: "Review", run: () => cockpit.filterTo({ show: "open" }) },
    { label: "Clear review filters", group: "Review", run: () => cockpit.clearFilters() },
    { label: "Done reviewing: summary to paste into a reply", group: "Review", run: showSummary },
    { label: `Show threads waiting on me (${cockpit.waitingOnMe().length})`, group: "Review", run: () => { cockpit.clearFilters(); cockpit.filterTo({ show: "mine" }); } },
    { label: "Triage threads waiting on me", group: "Review", run: () => { cockpit.clearFilters(); cockpit.filterTo({ show: "mine" }); return cockpit.startTriage(); } },
    { label: `${sinceOn ? "Hide" : "Show"} what changed since I last saw it`, group: "Review", hint: "Alt+S", run: () => toggleSince() },
    { label: `Track changes: turn ${tracking ? "off" : "on"}`, group: "Command", run: toggleTrack },
    {
      label: `Spell checking: turn ${spellOn() ? "off" : "on"}`,
      group: "Command",
      run: () => {
        localStorage.setItem("docex-spell", spellOn() ? "off" : "");
        spellWarned = "";
        void applySpellcheck();
      },
    },
    { label: "Save", group: "Command", hint: "Ctrl+S", run: () => saveFile(false) },
    { label: "Save as…", group: "Command", hint: "Ctrl+Shift+S", run: () => saveFile(true) },
    { label: "Open…", group: "Command", hint: "Ctrl+O", run: () => openFile() },
    { label: "New document", group: "Command", hint: "Ctrl+N", run: newFile },
    { label: "Toggle outline", group: "View", hint: "Ctrl+1", run: () => togglePanel("outline") },
    { label: "Toggle review panel", group: "View", hint: "Ctrl+2", run: () => togglePanel("sidebar") },
    { label: "Find…", group: "Command", hint: "Ctrl+F", run: () => findBar.open(false) },
    { label: "Find and replace…", group: "Command", hint: "Ctrl+H", run: () => findBar.open(true) },
    { label: "Insert table", group: "Insert", run: insertTable },
    ...TABLE_CMDS.map(([, tip, cmd]) => ({ label: `Table: ${tip.replace(/\s+\(.*\)$/, "").toLowerCase()}`, group: "Table", run: () => { cmd(view.state, view.dispatch); view.focus(); } })),
    { label: "Insert image…", group: "Insert", run: insertImage },
    { label: "Insert footnote", group: "Insert", run: insertFootnote },
    { label: "Insert formula", group: "Insert", run: () => insertMath() },
    { label: "Comment on selection", group: "Insert", run: addComment },
    ...KINDS.map(([k, label]) => ({ label: `Style: ${label}`, group: "Format", run: () => { setKind(k)(view.state, view.dispatch); view.focus(); } })),
    ...formatEntries(),
    ...aiEntries(),
  ];
  idx.sections.forEach((s, i) => {
    if (s.level > 0) e.push({ label: `${"  ".repeat(Math.max(0, s.level - 1))}${s.title}`, group: "Go to", keywords: "section heading", hint: s.changes + s.comments ? `${s.changes}·${s.comments}` : undefined, run: () => cockpit.scrollToSection(i) });
  });
  for (const p of idx.people) {
    const mine = idx.items.filter((i) => i.author === p.name && i.kind !== "comment");
    e.push({ label: `Show only ${p.name}`, group: "Person", run: () => cockpit.filterTo({ person: p.name }) });
    if (mine.length) {
      e.push({ label: `Triage ${p.name}'s changes (${mine.length})`, group: "Person", run: () => { cockpit.clearFilters(); cockpit.filterTo({ person: p.name, show: "changes" }); return cockpit.startTriage(); } });
      e.push({ label: `Accept all ${mine.length} changes by ${p.name}`, group: "Person", run: () => cockpit.resolve(mine, true) });
      e.push({ label: `Reject all ${mine.length} changes by ${p.name}`, group: "Person", run: () => cockpit.resolve(mine, false) });
    }
  }
  for (const c of idx.clusters.slice(0, 30)) {
    const what = `${c.old ? `"${c.old}"` : "∅"} → ${c.new ? `"${c.new}"` : "∅"}`;
    e.push({ label: `Show ×${c.items.length} ${what}`, group: "Repeated edit", run: () => cockpit.filterTo({ cluster: c.key }) });
    e.push({ label: `Accept all ×${c.items.length} ${what}`, group: "Repeated edit", run: () => cockpit.resolve(c.items, true) });
  }
  for (const r of recentFiles()) if (r !== current?.path) e.push({ label: basename(r), group: "Recent", hint: r.replace(/\/[^/]*$/, ""), run: async () => {
    if (await confirmDiscard()) await openFile(r);
  } });
  return e;
}

// ---------------------------------------------------------------------------
// Startup

document.addEventListener("keydown", (e) => {
  if (e.altKey && !e.ctrlKey && !e.metaKey && (e.key.toLowerCase() === "t" || e.key.toLowerCase() === "s")) {
    e.preventDefault();
    if (e.key.toLowerCase() === "t") void cockpit.startTriage();
    else toggleSince();
    return;
  }
  const mod = e.ctrlKey || e.metaKey;
  if (!mod) return;
  const k = e.key.toLowerCase();
  if (k === "s") {
    e.preventDefault();
    void saveFile(e.shiftKey);
  } else if (k === "o") {
    e.preventDefault();
    void openFile();
  } else if (k === "n") {
    e.preventDefault();
    void newFile();
  } else if (k === "1" || k === "2") {
    e.preventDefault();
    togglePanel(k === "1" ? "outline" : "sidebar");
  } else if (k === "p") {
    e.preventDefault();
    openPalette(paletteEntries());
  } else if (k === "f" || k === "h") {
    e.preventDefault();
    findBar.open(k === "h");
  } else if (k === "j") {
    e.preventDefault();
    openAIMenu();
  }
});
document.addEventListener("mousedown", (e) => {
  if (!(e.target as HTMLElement).closest("#popover, ins.chg, del.chg")) hidePopover();
});

void getCurrentWindow().onCloseRequested(async (e) => {
  if (!isDirty()) return;
  if (await ask("Close without saving?", { title: "docex", kind: "warning" })) dropRecovery();
  else e.preventDefault();
});

(async () => {
  for (const id of PANELS) panelPref[id] = !localStorage.getItem(`docex-hide-${id}`);
  layout();
  await initTheme(() => cockpit.refreshSoon());
  author = localStorage.getItem("docex-author") || (await invoke<string>("get_author"));
  await invoke("set_author", { name: author });
  const p = await invoke<string | null>("initial_path");
  if (p) await openFile(p);
  else await call("new_doc", {}, false);
  view.focus();
})();
