import "./style.css";
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
  Redo2,
  Save,
  SaveAll,
  Strikethrough,
  Table,
  Trash2,
  Undo2,
} from "lucide";
import { initTheme } from "./theme";
import { atomIndex, docToModel, modelToDoc, type PendingComment } from "./convert";
import { imageUrls, KINDS, schema } from "./schema";
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
let sideTab: "changes" | "comments" = "changes";

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
      Tab: (s, d) => (isInTable(s) ? goToNextCell(1)(s, d) : shiftLevel(1)(s, d) || true),
      "Shift-Tab": (s, d) => (isInTable(s) ? goToNextCell(-1)(s, d) : shiftLevel(-1)(s, d) || true),
    }),
    keymap(baseKeymap),
    dropCursor(),
    gapCursor(),
    tableEditing(),
  ];
}

const view = new EditorView($("#editor"), {
  state: EditorState.create({ doc: schema.nodes.doc.create(null, schema.nodes.para.create()), plugins: plugins() }),
  dispatchTransaction(tr: Transaction) {
    let t = tr;
    if (tracking) t = trackTransaction(view.state, tr, author) ?? tr;
    view.updateState(view.state.apply(t));
    if (t.docChanged && !localDirty) {
      localDirty = true;
      renderStatus();
    }
    updateToolbar();
  },
  handleDOMEvents: {
    click(_v, e) {
      const el = (e.target as HTMLElement).closest("ins.chg, del.chg") as HTMLElement | null;
      if (el && el.dataset.wid) showChangePopover(el);
      else hidePopover();
      const c = (e.target as HTMLElement).closest(".cmt") as HTMLElement | null;
      if (c) highlightComment(c.dataset.ids?.split(" ")[0] || "", false);
      return false;
    },
  },
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
  renderSidebar();
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
  sideTab = "comments";
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

// ---------------------------------------------------------------------------
// Sidebar

const KIND_LABEL: Record<string, string> = {
  insert: "Inserted",
  delete: "Deleted",
  "move-to": "Moved here",
  "move-from": "Moved away",
  "paragraph-insert": "New paragraph",
  "paragraph-delete": "Removed paragraph",
  format: "Formatting",
  "paragraph-format": "Paragraph style",
  "row-insert": "New table row",
  "row-delete": "Removed table row",
  "table-format": "Table formatting",
  "section-format": "Page setup",
};

function scrollToSelector(sel: string) {
  const el = view.dom.querySelector(sel) as HTMLElement | null;
  if (!el) return false;
  el.scrollIntoView({ block: "center", behavior: "smooth" });
  el.classList.add("flash");
  setTimeout(() => el.classList.remove("flash"), 1200);
  return true;
}

function highlightComment(id: string, scroll = true) {
  document.querySelectorAll(".comment-card.active").forEach((e) => e.classList.remove("active"));
  const card = document.querySelector(`.comment-card[data-id="${id}"]`);
  if (card) {
    if (sideTab !== "comments") {
      sideTab = "comments";
      renderSidebar();
      return highlightComment(id, scroll);
    }
    card.classList.add("active");
    card.scrollIntoView({ block: "nearest" });
  }
  if (scroll) scrollToSelector(`.cmt[data-ids~="${id}"]`);
}

function changeCard(c: Change) {
  const card = h(
    "div",
    { class: `change-card ${c.kind}` },
    h("div", { class: "meta" }, h("b", {}, KIND_LABEL[c.kind] ?? c.kind), ` · ${c.author || "unknown"}`, c.part !== "body" ? ` · ${c.part}` : ""),
    h("div", { class: "date" }, shortDate(c.date)),
    c.text && c.text !== "¶" ? h("div", { class: "excerpt" }, c.text) : "",
    h(
      "div",
      { class: "row" },
      h("button", { textContent: "Accept", onclick: (e: Event) => { e.stopPropagation(); void accept(c.wids, true); } }),
      h("button", { textContent: "Reject", onclick: (e: Event) => { e.stopPropagation(); void accept(c.wids, false); } }),
    ),
  );
  card.onclick = () => c.wids.some((w) => scrollToSelector(`[data-wid="${w}"]`));
  return card;
}

function commentCard(c: Comment, replies: Comment[]) {
  const card = h("div", { class: `comment-card${c.done ? " done" : ""}` });
  card.dataset.id = c.id;
  const one = (x: Comment, reply: boolean) =>
    h(
      "div",
      { class: reply ? "reply" : "" },
      h("div", { class: "meta" }, h("b", {}, x.author || "unknown"), ` · ${shortDate(x.date)}`),
      h("div", { class: "text" }, x.text),
    );
  if (c.anchor) card.append(h("div", { class: "anchor" }, `“${c.anchor.length > 90 ? c.anchor.slice(0, 88) + "…" : c.anchor}”`));
  card.append(one(c, false));
  for (const r of replies) card.append(one(r, true));
  card.append(
    h(
      "div",
      { class: "row" },
      h("button", {
        textContent: "Reply",
        onclick: async (e: Event) => {
          e.stopPropagation();
          const text = await askText(`Reply to ${c.author}`, "", true);
          if (text?.trim()) await call("comment_reply", { id: c.id, text: text.trim() });
        },
      }),
      h("button", { textContent: c.done ? "Reopen" : "Resolve", onclick: (e: Event) => { e.stopPropagation(); void call("comment_done", { id: c.id, done: !c.done }); } }),
      h("button", {
        textContent: "Delete",
        onclick: async (e: Event) => {
          e.stopPropagation();
          if (await ask("Delete this comment and its replies?", { title: "docex", kind: "warning" })) await call("comment_delete", { id: c.id });
        },
      }),
    ),
  );
  card.onclick = () => highlightComment(c.id);
  return card;
}

function renderSidebar() {
  const changes = current?.changes ?? [];
  const comments = current?.comments ?? [];
  const tops = comments.filter((c) => !c.parent);
  const tab = (id: typeof sideTab, label: string, n: number) =>
    h("button", { class: `tab${sideTab === id ? " active" : ""}`, onclick: () => { sideTab = id; renderSidebar(); } }, `${label} `, h("span", { class: "count" }, String(n)));
  const body = h("div", { class: "side-body" });
  if (sideTab === "changes") {
    body.append(
      h(
        "div",
        { class: "row side-actions" },
        h("button", { textContent: "Accept all", disabled: !changes.length, onclick: () => call("resolve_all", { accept: true }) }),
        h("button", { textContent: "Reject all", disabled: !changes.length, onclick: () => call("resolve_all", { accept: false }) }),
        h("button", { textContent: "Refresh", title: "Turn your edits into tracked changes now", onclick: () => call("sync") }),
      ),
    );
    if (!changes.length) body.append(h("p", { class: "empty" }, "No tracked changes."));
    if (localDirty) body.append(h("p", { class: "hint" }, "Your latest edits show up here after Refresh or Save."));
    for (const c of changes) body.append(changeCard(c));
  } else {
    body.append(h("div", { class: "row side-actions" }, h("button", { textContent: "Comment on selection", onclick: addComment })));
    if (!tops.length) body.append(h("p", { class: "empty" }, "No comments."));
    for (const c of tops) body.append(commentCard(c, comments.filter((r) => r.parent === c.id)));
  }
  $("#sidebar").replaceChildren(h("div", { class: "tabs" }, tab("changes", "Changes", changes.length), tab("comments", "Comments", tops.length)), body);
}

function renderStatus() {
  const s = $("#status");
  const dirty = isDirty();
  s.replaceChildren(
    h("span", { class: "path", title: current?.path ?? "" }, current?.path ?? "Untitled"),
    dirty ? h("span", { class: "dirty" }, "● unsaved") : "",
    h("span", { class: "spacer" }),
    h("span", {}, tracking ? "tracking changes" : "editing directly"),
    h("button", {
      class: "author",
      title: "Name used for your changes and comments",
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

const kindSelect = h("select", { title: "Paragraph style" });
for (const [k, label] of KINDS) kindSelect.append(h("option", { value: k }, label));
kindSelect.onchange = () => {
  setKind(kindSelect.value)(view.state, view.dispatch);
  view.focus();
};
const styles = h("span", { class: "styles" }, kindSelect, icon(ChevronDown));

const markButtons: [HTMLButtonElement, MarkType][] = [
  [tb(Bold, "Bold  Ctrl+B", () => toggleMark(schema.marks.strong)(view.state, view.dispatch)), schema.marks.strong],
  [tb(Italic, "Italic  Ctrl+I", () => toggleMark(schema.marks.em)(view.state, view.dispatch)), schema.marks.em],
  [tb(Strikethrough, "Strikethrough", () => toggleMark(schema.marks.strike)(view.state, view.dispatch)), schema.marks.strike],
  [tb(Code, "Code  Ctrl+`", () => toggleMark(schema.marks.code)(view.state, view.dispatch)), schema.marks.code],
  [tb(Link, "Link  Ctrl+K", () => toggleLink(view.state, view.dispatch)), schema.marks.link],
];

const saveBtn = tb(Save, "Save  Ctrl+S", () => saveFile(false));
const tableCtx = h(
  "span",
  { class: "tb-ctx" },
  sep(),
  tb(BetweenHorizontalEnd, "Add row below", () => addRowAfter(view.state, view.dispatch)),
  tb(Trash2, "Remove row", () => deleteRow(view.state, view.dispatch)),
);

const trackSwitch = h("button", { class: "track-switch", onclick: () => void toggleTrack() }, h("span", { class: "knob" }), h("span", { class: "lbl" }, "Track changes"));
trackSwitch.dataset.tip = "Word's Track Changes setting, saved in the document";

$("#toolbar").append(
  h("span", { class: "brand" }, "docex"),
  tb(FilePlus, "New  Ctrl+N", newFile),
  tb(FolderOpen, "Open  Ctrl+O", () => openFile()),
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
  tb(Image, "Insert image", insertImage),
  tb(Table, "Insert table", insertTable),
  tb(Asterisk, "Insert footnote", insertFootnote),
  tb(MessageSquarePlus, "Comment on selection", addComment),
  tableCtx,
  h("span", { class: "grow" }),
  trackSwitch,
);

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
  }
});
document.addEventListener("mousedown", (e) => {
  if (!(e.target as HTMLElement).closest("#popover, ins.chg, del.chg")) hidePopover();
});

void getCurrentWindow().onCloseRequested(async (e) => {
  if (isDirty() && !(await ask("Close without saving?", { title: "docex", kind: "warning" }))) e.preventDefault();
});

(async () => {
  await initTheme();
  author = localStorage.getItem("docex-author") || (await invoke<string>("get_author"));
  await invoke("set_author", { name: author });
  const p = await invoke<string | null>("initial_path");
  if (p) await openFile(p);
  else await call("new_doc", {}, false);
  view.focus();
})();
