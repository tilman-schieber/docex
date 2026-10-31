// Pictures in the editor: shown at their size on the page, resized by
// dragging a corner, replaced, saved and described (alt text, by hand or by
// the AI). Pasted and dropped pictures are inserted; a dropped .docx opens.
//
// Word doesn't track a picture's alt text or size, so those change directly;
// replacing, inserting and deleting a picture are tracked like text.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { Node as PMNode } from "prosemirror-model";
import type { EditorView, NodeView } from "prosemirror-view";
import { runAI } from "./ai";
import { imageUrls, schema } from "./schema";

/** EMU per CSS pixel (96 per inch). */
export const EMU_PX = 9525;
/** What Word documents take as they are; other formats are converted to PNG. */
const INSERTABLE = ["png", "jpg", "jpeg", "gif"];
const PICTURES = [...INSERTABLE, "webp", "bmp", "svg"];
const DOCUMENTS = ["docx", "dotx", "docm"];

export interface ImageHost {
  view: EditorView;
  toast(msg: string, kind?: "info" | "error", action?: { label: string; run: () => unknown }): void;
  progress(msg: string): { set(m: string): void; done(): void };
  askText(title: string, initial?: string, multiline?: boolean): Promise<string | null>;
  /** Accept / reject buttons when the picture is part of a tracked change. */
  changeRow(node: PMNode): HTMLElement | null;
  /** The document's language (for alt text), if known. */
  lang(): string | null;
  openFile(path: string): unknown;
}

const ext = (p: string) => (p.split(/[?#]/)[0].split(".").pop() ?? "").toLowerCase();
const basename = (p: string) => p.split(/[\\/]/).pop() || p;
const cm = (emu: number) => (emu / 360000).toFixed(1).replace(/\.0$/, "");

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", ...kids: (Node | string | null | false)[]) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  for (const k of kids) if (k) e.append(k);
  return e;
}

function button(label: string, tip: string, run: () => unknown, cls = "") {
  const b = el("button", cls, label);
  b.setAttribute("data-tip", tip);
  b.onclick = () => void run();
  return b;
}

/** A dashed box naming a picture the editor can't show (EMF, WMF, TIFF). */
function placeholder(name: string, w: number, h: number) {
  const esc = (s: string) => s.replace(/[<&"]/g, (c) => `&#${c.charCodeAt(0)};`);
  const label = `No preview (${ext(name).toUpperCase() || "?"}): ${basename(name)}`;
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">` +
    `<rect x="1" y="1" width="${w - 2}" height="${h - 2}" fill="rgba(128,128,128,.08)" stroke="#888" stroke-dasharray="5 4"/>` +
    `<text x="50%" y="50%" text-anchor="middle" dominant-baseline="middle" font-family="sans-serif" font-size="13" fill="#888">${esc(label)}</text></svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("can't read this picture"));
    img.src = url;
  });
}

/** A picture redrawn as a data URL: PNG, or JPEG for photos; at most `max` pixels on its long side. */
function redraw(img: HTMLImageElement, max = Infinity, jpeg = false) {
  const s = Math.min(1, max / Math.max(img.naturalWidth, img.naturalHeight));
  const c = document.createElement("canvas");
  c.width = Math.max(1, Math.round(img.naturalWidth * s));
  c.height = Math.max(1, Math.round(img.naturalHeight * s));
  c.getContext("2d")!.drawImage(img, 0, 0, c.width, c.height);
  return jpeg ? c.toDataURL("image/jpeg", 0.9) : c.toDataURL("image/png");
}

const base64 = (url: string) => url.slice(url.indexOf(",") + 1);

// ---------------------------------------------------------------------------
// The picture on the page

class ImageView implements NodeView {
  dom: HTMLElement;
  private img: HTMLImageElement;
  private grip: HTMLElement;

  constructor(
    private node: PMNode,
    private view: EditorView,
    private getPos: () => number | undefined,
  ) {
    this.dom = el("span", "imgv");
    this.img = el("img");
    this.img.draggable = false;
    this.img.addEventListener("error", () => {
      const { w, h } = this.size();
      if (!this.img.src.startsWith("data:image/svg")) this.img.src = placeholder(this.node.attrs.src, w, h);
    });
    this.grip = el("span", "imgv-grip");
    this.grip.setAttribute("data-tip", "Drag to resize");
    this.grip.addEventListener("mousedown", (e) => this.resize(e));
    this.dom.append(this.img, this.grip);
    this.render();
  }

  private size() {
    const { w, h } = this.node.attrs;
    return w && h ? { w: Math.round(w / EMU_PX), h: Math.round(h / EMU_PX) } : { w: 320, h: 160 };
  }

  private render() {
    const { src, alt, w } = this.node.attrs;
    const url = imageUrls.get(src);
    const { w: pw, h: ph } = this.size();
    if (this.img.dataset.src !== src || !this.img.src) {
      this.img.src = url || placeholder(src, pw, ph);
      this.img.dataset.src = src;
    }
    this.img.alt = alt;
    this.img.title = alt || basename(src);
    this.img.style.width = w ? `${pw}px` : "";
  }

  private resize(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    const x0 = e.clientX;
    const w0 = this.img.getBoundingClientRect().width;
    const ratio = this.img.naturalWidth ? this.img.naturalHeight / this.img.naturalWidth : this.size().h / this.size().w;
    const limit = (this.dom.closest("p, li, td, th, h1, h2, h3, h4, h5, h6, .ProseMirror") as HTMLElement | null)?.clientWidth ?? Infinity;
    let w = w0;
    this.dom.classList.add("resizing");
    const move = (m: MouseEvent) => {
      w = Math.max(16, Math.min(limit, w0 + m.clientX - x0));
      this.img.style.width = `${w}px`;
    };
    const up = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
      this.dom.classList.remove("resizing");
      const pos = this.getPos();
      if (pos === undefined || Math.abs(w - w0) < 1) return this.render();
      const emu = Math.round(w * EMU_PX);
      const tr = this.view.state.tr.setNodeAttribute(pos, "w", emu).setNodeAttribute(pos, "h", Math.round(emu * ratio));
      this.view.dispatch(tr);
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
  }

  update(node: PMNode) {
    if (node.type !== this.node.type) return false;
    this.node = node;
    this.render();
    return true;
  }

  selectNode() {
    this.dom.classList.add("ProseMirror-selectednode");
  }

  deselectNode() {
    this.dom.classList.remove("ProseMirror-selectednode");
  }

  stopEvent(e: Event) {
    return e.target === this.grip;
  }

  ignoreMutation() {
    return true;
  }
}

export const imageNodeView = (node: PMNode, view: EditorView, getPos: () => number | undefined) => new ImageView(node, view, getPos);

// ---------------------------------------------------------------------------
// Getting pictures in

/** A picture file ready to insert: its path (converted to PNG if needed) and pixel size. */
async function prepare(path: string): Promise<{ src: string; w: number; h: number }> {
  const url = imageUrls.get(path) ?? (await invoke<string>("image_data", { path }));
  const img = await loadImage(url).catch(() => {
    throw new Error(`${basename(path)}: docex can't read this picture`);
  });
  if (INSERTABLE.includes(ext(path))) {
    imageUrls.set(path, url);
    return { src: path, w: img.naturalWidth, h: img.naturalHeight };
  }
  // WebP, BMP, SVG: Word wants PNG.
  const png = redraw(img, ext(path) === "svg" ? 2400 : Infinity);
  const src = await invoke<string>("keep_pasted", { data: base64(png), ext: "png" });
  imageUrls.set(src, png);
  return { src, w: img.naturalWidth, h: img.naturalHeight };
}

/** Insert picture files at `pos` (else at the selection). */
export async function insertPictures(host: ImageHost, paths: string[], pos?: number) {
  const nodes: PMNode[] = [];
  for (const p of paths) {
    try {
      const { src } = await prepare(p);
      nodes.push(schema.nodes.image.create({ src, alt: "" }));
    } catch (e) {
      host.toast(String((e as Error).message ?? e), "error");
    }
  }
  if (!nodes.length) return;
  const { view } = host;
  const tr = pos === undefined ? view.state.tr.replaceSelectionWith(nodes[0]) : view.state.tr.insert(pos, nodes[0]);
  let at = tr.mapping.map(pos ?? view.state.selection.to);
  for (const n of nodes.slice(1)) {
    tr.insert(at, n);
    at += n.nodeSize;
  }
  view.dispatch(tr.scrollIntoView());
  view.focus();
}

/** Ask for picture files and insert them at the selection. */
export async function pickPictures(host: ImageHost) {
  const p = await openDialog({ multiple: true, filters: [{ name: "Pictures", extensions: PICTURES }] });
  if (!p) return;
  await insertPictures(host, Array.isArray(p) ? p : [p]);
}

/** Paste a picture from the clipboard (a screenshot, an image copied in a browser). */
export function pastePicture(host: ImageHost, e: ClipboardEvent): boolean {
  const data = e.clipboardData;
  const files = [...(data?.files ?? [])].filter((f) => f.type.startsWith("image/"));
  // Copied text (from Word, say) comes with a picture of itself: paste the text.
  if (!files.length || data?.getData("text/plain").trim()) return false;
  e.preventDefault();
  void (async () => {
    const paths: string[] = [];
    for (const f of files) {
      const url = await new Promise<string>((resolve) => {
        const r = new FileReader();
        r.onload = () => resolve(String(r.result));
        r.readAsDataURL(f);
      });
      try {
        const kind = f.type.split("/")[1];
        const keep = INSERTABLE.includes(kind) ? url : redraw(await loadImage(url));
        const path = await invoke<string>("keep_pasted", { data: base64(keep), ext: keep === url ? kind : "png" });
        imageUrls.set(path, keep);
        paths.push(path);
      } catch {
        host.toast("docex can't read the pasted picture.", "error");
      }
    }
    await insertPictures(host, paths);
  })();
  return true;
}

/** Files dropped on the window: a Word document opens, pictures go where they're dropped. */
export async function listenForDrops(host: ImageHost) {
  const page = document.getElementById("page-wrap");
  await getCurrentWebview().onDragDropEvent((e) => {
    const p = e.payload;
    page?.classList.toggle("dropping", p.type === "enter" || p.type === "over");
    if (p.type !== "drop") return;
    const doc = p.paths.find((f) => DOCUMENTS.includes(ext(f)));
    if (doc) return void host.openFile(doc);
    const pics = p.paths.filter((f) => PICTURES.includes(ext(f)));
    if (!pics.length) {
      if (p.paths.length) host.toast("docex opens Word documents and inserts PNG, JPEG, GIF, WebP, BMP and SVG pictures.");
      return;
    }
    const r = window.devicePixelRatio || 1;
    const at = host.view.posAtCoords({ left: p.position.x / r, top: p.position.y / r });
    void insertPictures(host, pics, at?.pos);
  });
}

// ---------------------------------------------------------------------------
// The picture menu

const pop = () => document.getElementById("popover")!;

export function hideImageMenu() {
  if (pop().dataset.image) {
    pop().hidden = true;
    delete pop().dataset.image;
  }
}

/** The picture at `pos`, still there? */
function nodeAt(view: EditorView, pos: number) {
  const n = view.state.doc.nodeAt(pos);
  return n?.type === schema.nodes.image ? n : null;
}

export function showImageMenu(host: ImageHost, pos: number) {
  const { view } = host;
  const node = nodeAt(view, pos);
  const dom = view.nodeDOM(pos) as HTMLElement | null;
  if (!node || !dom) return;
  const { src, alt, w, h } = node.attrs;
  const act = (fn: () => unknown) => () => {
    hideImageMenu();
    return fn();
  };
  const p = pop();
  p.replaceChildren(
    el("div", "pop-title", `Picture${w && h ? ` · ${cm(w)} × ${cm(h)} cm` : ""} · ${basename(src)}`),
    el("div", `pop-alt${alt ? "" : " none"}`, alt ? `“${alt}”` : "No alt text"),
    host.changeRow(node) ?? "",
    el(
      "div",
      "row",
      button("Replace…", "Another picture in its place, at the same width (tracked)", act(() => replacePicture(host, pos))),
      button("Alt text…", "Describe the picture for readers who can't see it", act(() => editAlt(host, pos))),
      button("Describe with AI", "Let the AI write the alt text; you check it before it goes in", act(() => describeOne(host, pos))),
    ),
    el(
      "div",
      "row",
      button("Save as…", "Save the picture as a file", act(() => savePicture(host, src))),
      button("Delete", "Delete the picture (tracked)", act(() => view.dispatch(view.state.tr.delete(pos, pos + 1))), "no"),
    ),
  );
  p.dataset.image = String(pos);
  const r = dom.getBoundingClientRect();
  p.style.left = "0px";
  p.hidden = false;
  p.style.left = `${Math.max(8, Math.min(r.left, window.innerWidth - p.offsetWidth - 8))}px`;
  p.style.top = `${Math.min(r.bottom + 6, window.innerHeight - p.offsetHeight - 8)}px`;
}

async function replacePicture(host: ImageHost, pos: number) {
  const p = await openDialog({ multiple: false, filters: [{ name: "Pictures", extensions: PICTURES }] });
  if (!p) return;
  try {
    const { src, w: pw, h: ph } = await prepare(p as string);
    const { view } = host;
    const old = nodeAt(view, pos);
    if (!old) return;
    // Same width on the page, the new picture's proportions, the same alt text.
    const w = old.attrs.w as number | null;
    const keep = old.marks.filter((m) => m.type !== schema.marks.ins && m.type !== schema.marks.del);
    const n = schema.nodes.image.create({ src, alt: old.attrs.alt, w, h: w ? Math.round((w * ph) / pw) : null }, null, keep);
    view.dispatch(view.state.tr.replaceWith(pos, pos + 1, n));
    view.focus();
  } catch (e) {
    host.toast(String((e as Error).message ?? e), "error");
  }
}

function setAlt(host: ImageHost, pos: number, alt: string) {
  const { view } = host;
  if (!nodeAt(view, pos)) return;
  view.dispatch(view.state.tr.setNodeAttribute(pos, "alt", alt.replace(/\s+/g, " ").trim()));
}

async function editAlt(host: ImageHost, pos: number, suggested?: string) {
  const node = nodeAt(host.view, pos);
  if (!node) return;
  const title = suggested === undefined ? "Alt text (for readers who can't see the picture)" : "Alt text written by the AI: check it";
  const alt = await host.askText(title, suggested ?? node.attrs.alt, true);
  if (alt !== null) setAlt(host, pos, alt);
}

async function savePicture(host: ImageHost, src: string) {
  const dest = await saveDialog({ defaultPath: basename(src), filters: [{ name: "Picture", extensions: [ext(src) || "png"] }] });
  if (!dest) return;
  try {
    await invoke("save_image", { src, dest });
    host.toast(`Saved ${basename(dest)}`);
  } catch (e) {
    host.toast(String(e), "error");
  }
}

/** Save every picture to a folder, numbered in reading order. */
export async function saveAllPictures(host: ImageHost) {
  const dir = await openDialog({ directory: true, title: "Save all pictures to…" });
  if (!dir) return;
  try {
    const n = await invoke<number>("save_images", { dir });
    host.toast(n ? `Saved ${n} picture${n === 1 ? "" : "s"} to ${dir}` : "This document has no pictures.");
  } catch (e) {
    host.toast(String(e), "error");
  }
}

// ---------------------------------------------------------------------------
// Alt text by the AI

export const ALT_SYSTEM = `You write alt text for pictures in Word documents, for readers who can't see them (screen readers, text-only copies).
- Say what the picture shows and why it is there, judging from the text around it. Leave out what the caption or the surrounding text already says.
- One or two plain sentences, under 150 characters. For a chart, diagram or table that carries information, up to 300: what it shows and the main values or trend.
- Don't start with "Image of" or "Picture of". No quotes, no Markdown.
- Write in the language of the document.
Answer with the alt text only.`;

/** Plain text of a block, shortened. */
const brief = (n: PMNode | null | undefined, max = 500) => {
  const t = (n?.textContent ?? "").replace(/\s+/g, " ").trim();
  return t.length > max ? `${t.slice(0, max)}…` : t;
};

/** What the AI needs: the picture (small enough to send) and the text around it. */
async function altRequest(host: ImageHost, pos: number) {
  const { doc } = host.view.state;
  const node = nodeAt(host.view, pos)!;
  const url = imageUrls.get(node.attrs.src);
  const img = url ? await loadImage(url).catch(() => null) : null;
  if (!img) return null;
  const picture = redraw(img, 1568, ["jpg", "jpeg"].includes(ext(node.attrs.src)));
  const $pos = doc.resolve(pos);
  const top = $pos.index(0);
  let heading = "";
  for (let i = top; i >= 0 && !heading; i--) {
    const b = doc.child(i);
    if (/^h[1-6]$/.test(b.attrs.kind ?? "")) heading = brief(b, 200);
  }
  const at = (i: number) => (i >= 0 && i < doc.childCount ? doc.child(i) : null);
  const next = at(top + 1);
  const caption = next?.attrs.kind === "caption" ? brief(next, 300) : "";
  const lines = [
    host.lang() ? `Document language: ${host.lang()}` : "",
    heading ? `Section: ${heading}` : "",
    caption ? `Caption: ${caption}` : "",
    `Text before: ${brief(at(top - 1)) || "(none)"}`,
    `Paragraph with the picture: ${brief($pos.parent) || "(only the picture)"}`,
    `Text after: ${brief(caption ? at(top + 2) : next) || "(none)"}`,
  ].filter(Boolean);
  return { picture, prompt: `Write the alt text for this picture.\n\n${lines.join("\n")}` };
}

/** The AI's alt text; null if the picture's format can't be read. */
async function describe(host: ImageHost, pos: number): Promise<string | null> {
  const req = await altRequest(host, pos);
  if (!req) return null;
  const out = await runAI(ALT_SYSTEM, req.prompt, undefined, 600, [req.picture]);
  return out.replace(/^["“]|["”]$/g, "").replace(/\s+/g, " ").trim() || null;
}

async function describeOne(host: ImageHost, pos: number) {
  const p = host.progress("The AI is looking at the picture…");
  try {
    const alt = await describe(host, pos);
    p.done();
    if (alt) await editAlt(host, pos, alt);
    else host.toast("The AI can't look at this picture: docex can't read its format.", "error");
  } catch (e) {
    p.done();
    host.toast(String(e), "error");
  }
}

/** Pictures without alt text (not deleted), as positions. */
export function picturesWithoutAlt(doc: PMNode) {
  const out: number[] = [];
  doc.descendants((n, pos) => {
    if (n.type === schema.nodes.image && !n.attrs.alt.trim() && !schema.marks.del.isInSet(n.marks)) out.push(pos);
  });
  return out;
}

/** The AI writes alt text for every picture without one; the user checks it all in one list. */
export async function describeAll(host: ImageHost) {
  const { view } = host;
  const todo = picturesWithoutAlt(view.state.doc).map((pos) => ({ pos, src: view.state.doc.nodeAt(pos)!.attrs.src as string }));
  if (!todo.length) return host.toast("Every picture has alt text.");
  const p = host.progress(`The AI is looking at the pictures: 0 of ${todo.length}…`);
  const found: { pos: number; src: string; alt: string }[] = [];
  let failed = 0;
  for (const [i, t] of todo.entries()) {
    try {
      const alt = await describe(host, t.pos);
      if (alt) found.push({ ...t, alt });
      else failed++;
    } catch (e) {
      p.done();
      return host.toast(String(e), "error");
    }
    p.set(`The AI is looking at the pictures: ${i + 1} of ${todo.length}…`);
  }
  p.done();
  if (!found.length) return;
  const rows = found.map((f) => {
    const ta = el("textarea");
    ta.value = f.alt;
    ta.rows = 3;
    const use = el("input");
    use.type = "checkbox";
    use.checked = true;
    const thumb = el("img", "alt-thumb");
    thumb.src = imageUrls.get(f.src) ?? "";
    return { f, ta, use, row: el("div", "alt-row", thumb, el("div", "alt-edit", ta, el("label", "check", use, " Use"))) };
  });
  const modal = document.getElementById("modal")!;
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
  const close = () => {
    modal.removeEventListener("keydown", onKey);
    modal.hidden = true;
    modal.replaceChildren();
    view.focus();
  };
  const apply = () => {
    // Positions still hold unless the text changed meanwhile; then find the picture again.
    const tr = view.state.tr;
    let n = 0;
    for (const { f, ta, use } of rows) {
      if (!use.checked || !ta.value.trim()) continue;
      let pos = f.pos;
      if (view.state.doc.nodeAt(pos)?.attrs.src !== f.src) {
        pos = -1;
        view.state.doc.descendants((nd, at) => {
          if (pos < 0 && nd.type === schema.nodes.image && nd.attrs.src === f.src && !nd.attrs.alt) pos = at;
        });
      }
      if (pos < 0) continue;
      tr.setNodeAttribute(pos, "alt", ta.value.replace(/\s+/g, " ").trim());
      n++;
    }
    close();
    if (n) {
      view.dispatch(tr);
      host.toast(`Alt text for ${n} picture${n === 1 ? "" : "s"}. Ctrl+Z takes it back.`);
    }
  };
  const ok = el("button", "primary", "Use these");
  ok.onclick = apply;
  const cancel = el("button", "", "Cancel");
  cancel.onclick = close;
  modal.replaceChildren(
    el(
      "div",
      "dialog wide alt-dialog",
      el("label", "", `Alt text written by the AI for ${found.length} picture${found.length === 1 ? "" : "s"}: check and edit`),
      failed ? el("div", "hint", `${failed} more couldn't be described (their format can't be read).`) : null,
      el("div", "alt-list", ...rows.map((r) => r.row)),
      el("div", "row", cancel, ok),
    ),
  );
  modal.hidden = false;
  modal.addEventListener("keydown", onKey);
  rows[0]?.ta.focus();
}
