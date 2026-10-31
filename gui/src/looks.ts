// The document's look in the editor: its fonts, spacing and page as its
// styles say (so the page looks as it will in Word), and the picker that
// gives it another look: a built-in theme, a .css file, or another
// document's look. Only Word's styles and the page change, never the text.

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

export interface LookHost {
  toast(msg: string, kind?: "info" | "error"): void;
  /** Apply a theme (built-in name or file path) to the document. */
  apply(theme: string): Promise<unknown>;
  /** The page changed size: re-measure what depends on it. */
  relayout(): void;
}

interface Theme {
  name: string;
  about: string;
  look: string;
}

const OFF = "docex-look-off";
const PRINT = "docex-print";

const style = document.createElement("style");
style.id = "doc-look";
document.head.append(style);
let doc = "";

const page = () => document.getElementById("editor")!;

const pref = (k: string) => {
  try {
    return localStorage.getItem(k) === "1";
  } catch {
    return false;
  }
};

const setPref = (k: string, on: boolean) => {
  try {
    localStorage.setItem(k, on ? "1" : "");
  } catch {
    /* only for this session */
  }
};

export const lookOn = () => !pref(OFF);
export const printOn = () => pref(PRINT);

function render(css: string, force = false) {
  style.textContent = css;
  page().classList.toggle("look", !!css && (force || lookOn()));
  page().classList.toggle("print", printOn());
}

/** The open document's look (CSS from the backend). */
export function showLook(css: string) {
  doc = css;
  if (!picker) render(css);
}

export function setLookOn(host: LookHost, on: boolean) {
  setPref(OFF, !on);
  render(doc);
  host.relayout();
}

export function setPrint(host: LookHost, on: boolean) {
  setPref(PRINT, on);
  render(picker ? style.textContent ?? doc : doc, !!picker);
  host.relayout();
}

export async function saveLook(host: LookHost) {
  const dest = await saveDialog({ defaultPath: "look.css", filters: [{ name: "Stylesheet", extensions: ["css"] }] });
  if (!dest) return;
  try {
    await invoke("save_theme", { dest });
    host.toast(`Saved the look as ${dest.split(/[\\/]/).pop()}. Edit it and apply it to any document.`);
  } catch (e) {
    host.toast(String(e), "error");
  }
}

let picker: HTMLElement | null = null;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", ...kids: (Node | string | null | false)[]) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  for (const k of kids) if (k) e.append(k);
  return e;
}

/** A panel beside the page: pick a look and see it on the page, then apply or go back. */
export async function openPicker(host: LookHost) {
  if (picker) return;
  let themes: Theme[];
  try {
    themes = await invoke<Theme[]>("themes");
  } catch (e) {
    host.toast(String(e), "error");
    return;
  }
  type Choice = { spec: string | null; name: string; about: string; look: string };
  const choices: Choice[] = [
    { spec: null, name: "As it is", about: "The document's own styles", look: doc },
    ...themes.map((t) => ({ spec: t.name, name: t.name[0].toUpperCase() + t.name.slice(1), about: t.about, look: t.look })),
  ];
  let sel = 0;
  const list = el("div", "lk-list");
  const note = el("div", "lk-note");
  const apply = el("button", "primary", "Apply");
  const close = () => {
    picker?.remove();
    picker = null;
    document.removeEventListener("keydown", onKey, true);
    render(doc);
    host.relayout();
  };
  const choose = (i: number) => {
    sel = Math.max(0, Math.min(choices.length - 1, i));
    for (const [k, row] of [...list.children].entries()) row.classList.toggle("sel", k === sel);
    render(choices[sel].look, true);
    apply.disabled = choices[sel].spec === null;
    host.relayout();
  };
  const fromFile = async () => {
    const p = await openDialog({ multiple: false, filters: [{ name: "Stylesheet or Word document", extensions: ["css", "docx", "dotx"] }] });
    if (!p) return;
    const path = p as string;
    try {
      const r = await invoke<{ look: string; warnings: string[] }>("theme_look", { theme: path });
      choices.push({ spec: path, name: path.split(/[\\/]/).pop()!, about: /\.(docx|dotx)$/i.test(path) ? "The look of this document" : "Your stylesheet", look: r.look });
      draw();
      choose(choices.length - 1);
      note.textContent = r.warnings.length ? `Ignored: ${r.warnings.join("; ")}` : "";
    } catch (e) {
      host.toast(String(e), "error");
    }
  };
  const draw = () => {
    list.replaceChildren(
      ...choices.map((c, i) => {
        const row = el("button", "lk-row", el("span", "lk-name", c.name), el("span", "lk-about", c.about));
        row.onclick = () => choose(i);
        row.ondblclick = () => void go();
        return row;
      }),
    );
  };
  const go = async () => {
    const c = choices[sel];
    if (c.spec === null) return close();
    close();
    await host.apply(c.spec);
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
    else if (e.key === "ArrowDown") choose(sel + 1);
    else if (e.key === "ArrowUp") choose(sel - 1);
    else if (e.key === "Enter") void go();
    else return;
    e.preventDefault();
    e.stopPropagation();
  };
  const print = el("input");
  print.type = "checkbox";
  print.checked = printOn();
  print.onchange = () => setPrint(host, print.checked);
  const other = el("button", "lk-file", "From a file… (.css, or a .docx to look like)");
  other.onclick = () => void fromFile();
  const cancel = el("button", "", "Cancel");
  cancel.onclick = close;
  apply.onclick = () => void go();
  picker = el(
    "div",
    "",
    el("div", "lk-head", "Document look"),
    el("div", "lk-hint", "Fonts, sizes, spacing, colours and page, through Word's own styles. The text stays as it is."),
    list,
    other,
    note,
    el("label", "check", print, " Print colours (white page)"),
    el("div", "row", cancel, apply),
  );
  picker.id = "looks";
  document.body.append(picker);
  draw();
  choose(0);
  document.addEventListener("keydown", onKey, true);
}
