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
import { addRowAfter, deleteRow, goToNextCell, isInTable, tableEditing } from "prosemirror-tables";
import { EditorView } from "prosemirror-view";
import {
  Asterisk,
  BetweenHorizontalEnd,
  Bold,
  ChevronDown,
  Code,
  createElement,
  FilePlus,
  FolderOpen,
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
  Table,
  Trash2,
  Undo2,
} from "lucide";
import { initTheme } from "./theme";
import { highlightPlugin } from "./highlight";
import { markdownBackspace, markdownRules, pasteMarkdown } from "./markdown";
import { createCockpit } from "./cockpit";
import { atomIndex, docToModel, modelToDoc, type PendingComment } from "./convert";
import { imageUrls, KINDS, renderMath, schema } from "./schema";
import { trackTransaction } from "./track";
import type { Change, Comment, View } from "./types";

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

function toast(msg: string, kind: "info" | "error" = "info") {
  const t = h("div", { class: `toast ${kind}` }, msg);
  $("#toasts").append(t);
  setTimeout(() => t.remove(), kind === "error" ? 8000 : 4000);
}

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
      "Mod-z": undo,
      "Mod-y": redo,
      "Shift-Mod-z": redo,
      "Mod-b": toggleMark(schema.marks.strong),
      "Mod-i": toggleMark(schema.marks.em),
      "Mod-`": toggleMark(schema.marks.code),
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
  ];
}

const view = new EditorView($("#editor"), {
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

const cockpit = createCockpit({
  view,
  current: () => current,
  me: () => author,
  call: (cmd, args) => call(cmd, args),
  toast,
});
cockpit.onSectionChange((t) => {
  sectionTitle = t;
  renderStatus();
});

function load(v: View, keepSelection = true) {
  current = v;
  tracking = v.track;
  for (const [k, url] of Object.entries(v.images)) imageUrls.set(k, url);
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
  localDirty = false;
  for (const w of v.warnings) toast(w);
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
    const payload = withModel ? { model: model(args.pending as PendingComment | undefined), tracked: tracking, ...args } : args;
    delete (payload as Record<string, unknown>).pending;
    const v = await invoke<View>(cmd, payload);
    load(v);
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
  return ask("Discard unsaved changes?", { title: "docex", kind: "warning" });
}

async function openFile(path?: string) {
  if (!path) {
    if (!(await confirmDiscard())) return;
    const p = await openDialog({ multiple: false, filters: [{ name: "Word document", extensions: ["docx"] }] });
    if (!p) return;
    path = p as string;
  }
  await call("open_doc", { path }, false);
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
  if (v) toast(`Saved ${basename(v.path || "")}`);
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

const accept = (wids: string[], yes: boolean) => call("resolve", { wids, accept: yes });

// ---------------------------------------------------------------------------
// Popover for tracked changes in the text

function hidePopover() {
  $("#popover").hidden = true;
}

function showChangePopover(el: HTMLElement) {
  const pop = $("#popover");
  const wid = el.dataset.wid!;
  const ch = current?.changes.find((c) => c.wids.includes(wid));
  pop.replaceChildren(
    h("div", { class: "pop-title" }, el.title),
    h(
      "div",
      { class: "row" },
      h("button", { textContent: "Accept", onclick: () => { hidePopover(); void accept(ch ? ch.wids : [wid], true); } }),
      h("button", { textContent: "Reject", onclick: () => { hidePopover(); void accept(ch ? ch.wids : [wid], false); } }),
    ),
  );
  const r = el.getBoundingClientRect();
  pop.style.left = `${Math.max(8, r.left)}px`;
  pop.style.top = `${r.bottom + 6}px`;
  pop.hidden = false;
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
  [tb(Link, "Link  Ctrl+K", () => toggleLink(view.state, view.dispatch)), schema.marks.link],
];

const saveBtn = tb(Save, "Save  Ctrl+S", () => saveFile(false));
const tableCtx = h(
  "span",
  { class: "tb-ctx" },
  sep(),
  tb(BetweenHorizontalEnd, "Add a table row below", () => addRowAfter(view.state, view.dispatch)),
  tb(Trash2, "Remove this table row", () => deleteRow(view.state, view.dispatch)),
);

const trackSwitch = h("button", { class: "track-switch", onclick: () => void toggleTrack() }, h("span", { class: "knob" }), h("span", { class: "lbl" }, "Track changes"));
trackSwitch.dataset.tip = "Track Changes: record your edits as suggestions others can accept or reject\n(Word's setting, saved in the document)";

$("#toolbar").append(
  h("span", { class: "brand" }, "docex"),
  tb(FilePlus, "New document  Ctrl+N", newFile),
  tb(FolderOpen, "Open a .docx  Ctrl+O", () => openFile()),
  saveBtn,
  tb(SaveAll, "Save as  Ctrl+Shift+S", () => saveFile(true)),
  sep(),
  tb(Undo2, "Undo  Ctrl+Z", () => undo(view.state, view.dispatch)),
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
  tableCtx.classList.toggle("show", isInTable(view.state));
  trackSwitch.classList.toggle("on", tracking);
  saveBtn.querySelector(".dot")?.remove();
  if (isDirty()) saveBtn.append(h("span", { class: "dot" }));
}

// ---------------------------------------------------------------------------
// Startup

document.addEventListener("keydown", (e) => {
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
  }
});
document.addEventListener("mousedown", (e) => {
  if (!(e.target as HTMLElement).closest("#popover, ins.chg, del.chg")) hidePopover();
});

void getCurrentWindow().onCloseRequested(async (e) => {
  if (isDirty() && !(await ask("Close without saving?", { title: "docex", kind: "warning" }))) e.preventDefault();
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
