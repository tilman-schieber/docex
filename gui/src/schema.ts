import { Schema, type DOMOutputSpec, type Mark, type Node as PMNode } from "prosemirror-model";
import { tableNodes } from "prosemirror-tables";
import katex from "katex";

/** Typeset LaTeX into `el` (shows the source if it doesn't parse). */
export function renderMath(el: HTMLElement, latex: string, display: boolean) {
  try {
    katex.render(latex || "\\square", el, { displayMode: display, throwOnError: false, strict: "ignore" });
  } catch {
    el.textContent = latex;
  }
}

/** Data URLs for images, by the `src` stored in image nodes. */
export const imageUrls = new Map<string, string>();

export const KINDS: [string, string][] = [
  ["normal", "Normal"],
  ["h1", "Heading 1"],
  ["h2", "Heading 2"],
  ["h3", "Heading 3"],
  ["h4", "Heading 4"],
  ["bullet", "Bulleted list"],
  ["number", "Numbered list"],
  ["quote", "Quote"],
  ["code", "Code"],
  ["caption", "Caption"],
];

const OBJ_LABELS: Record<string, string> = {
  "<!--docex:shape-->": "shape",
  "<!--docex:drawing-->": "drawing",
  "<!--docex:object-->": "object",
  "<!--docex:math-->": "equation",
  "<!--docex:pagebreak-->": "page break",
  "<!--docex:columnbreak-->": "column break",
  "<!--docex:sym-->": "symbol",
};

const changeAttrs = { author: { default: "" }, date: { default: "" }, wid: { default: "" } };

function changeTitle(kind: string, m: Mark): string {
  const who = m.attrs.author || "you";
  const when = m.attrs.date ? ` · ${m.attrs.date.replace("T", " ").replace("Z", "")}` : "";
  return `${kind} by ${who}${when}`;
}

export const schema = new Schema({
  nodes: {
    doc: { content: "block+" },
    para: {
      // `label`: Word's automatic number ("3.2", "(b)"), shown but never saved.
      attrs: { kind: { default: "normal" }, level: { default: 0 }, label: { default: "" } },
      content: "inline*",
      group: "block",
      parseDOM: [
        { tag: "p", getAttrs: (d) => ({ kind: (d as HTMLElement).dataset.kind || "normal", level: Number((d as HTMLElement).dataset.level || 0) }) },
        ...[1, 2, 3, 4, 5, 6].map((n) => ({ tag: `h${n}`, attrs: { kind: `h${n}` } })),
        { tag: "li", attrs: { kind: "bullet" } },
        { tag: "blockquote", attrs: { kind: "quote" } },
        { tag: "pre", preserveWhitespace: "full" as const, attrs: { kind: "code" } },
      ],
      toDOM(n): DOMOutputSpec {
        const { kind, level, label } = n.attrs;
        const tag = /^h[1-6]$/.test(kind) ? kind : "p";
        const attrs: Record<string, string> = { class: `k-${kind} lvl-${level}${label ? " has-label" : ""}`, "data-kind": kind, "data-level": String(level) };
        if (label) attrs["data-label"] = label;
        return [tag, attrs, 0];
      },
    },
    text: { group: "inline" },
    hard_break: {
      inline: true,
      group: "inline",
      selectable: false,
      parseDOM: [{ tag: "br" }],
      toDOM: () => ["br"],
    },
    image: {
      inline: true,
      group: "inline",
      atom: true,
      draggable: true,
      attrs: { src: {}, alt: { default: "" } },
      parseDOM: [],
      toDOM(n) {
        return ["img", { src: imageUrls.get(n.attrs.src) || "", alt: n.attrs.alt, title: n.attrs.alt || n.attrs.src, "data-src": n.attrs.src }];
      },
    },
    math: {
      inline: true,
      group: "inline",
      atom: true,
      attrs: { latex: {}, display: { default: false } },
      toDOM(n) {
        const el = document.createElement("span");
        el.className = n.attrs.display ? "math display" : "math";
        el.dataset.latex = n.attrs.latex;
        el.title = "Click to edit the formula";
        renderMath(el, n.attrs.latex, n.attrs.display);
        return el;
      },
    },
    fnref: {
      inline: true,
      group: "inline",
      atom: true,
      attrs: { label: {} },
      toDOM(n) {
        const l: string = n.attrs.label;
        return ["sup", { class: "fnref", "data-label": l, title: `Footnote ${l}` }, /^\d+$/.test(l) ? l : "*"];
      },
    },
    obj: {
      inline: true,
      group: "inline",
      atom: true,
      attrs: { key: {} },
      toDOM(n) {
        const label = OBJ_LABELS[n.attrs.key] || n.attrs.key.replace(/<!--docex:|-->/g, "");
        return ["span", { class: "obj", title: "Not editable here; kept as it is" }, label];
      },
    },
    opaque: {
      group: "block",
      atom: true,
      selectable: true,
      attrs: { id: {}, label: { default: "" }, preview: { default: "" } },
      toDOM(n) {
        return [
          "div",
          { class: "opaque", contenteditable: "false" },
          ["div", { class: "opaque-label" }, n.attrs.label || "Content docex doesn't edit"],
          ["pre", { class: "opaque-preview" }, n.attrs.preview],
        ];
      },
    },
    textbox: {
      group: "block",
      content: "para+",
      attrs: { id: {} },
      isolating: true,
      toDOM: (n) => ["div", { class: "textbox", "data-id": n.attrs.id }, 0],
    },
    footnote: {
      group: "block",
      content: "para+",
      attrs: { label: {} },
      isolating: true,
      toDOM: (n) => {
        const l: string = n.attrs.label;
        return ["div", { class: "footnote", "data-label": /^\d+$/.test(l) ? l : "*" }, 0];
      },
    },
    ...tableNodes({ tableGroup: "block", cellContent: "para+", cellAttributes: {} }),
  },
  marks: {
    strong: { parseDOM: [{ tag: "strong" }, { tag: "b" }], toDOM: () => ["strong", 0] },
    em: { parseDOM: [{ tag: "em" }, { tag: "i" }], toDOM: () => ["em", 0] },
    strike: { parseDOM: [{ tag: "s" }], toDOM: () => ["s", 0] },
    code: { parseDOM: [{ tag: "code" }], toDOM: () => ["code", 0] },
    link: {
      attrs: { href: {} },
      inclusive: false,
      parseDOM: [{ tag: "a[href]", getAttrs: (d) => ({ href: (d as HTMLElement).getAttribute("href") }) }],
      toDOM: (m) => ["a", { href: m.attrs.href, title: m.attrs.href }, 0],
    },
    ins: {
      attrs: changeAttrs,
      inclusive: false,
      toDOM: (m) => ["ins", { class: m.attrs.wid ? "chg" : "chg mine", "data-wid": m.attrs.wid, "data-author": m.attrs.author, title: changeTitle("Inserted", m) }, 0],
    },
    del: {
      attrs: changeAttrs,
      inclusive: false,
      toDOM: (m) => ["del", { class: m.attrs.wid ? "chg" : "chg mine", "data-wid": m.attrs.wid, "data-author": m.attrs.author, title: changeTitle("Deleted", m) }, 0],
    },
    comment: {
      attrs: { ids: {} },
      inclusive: false,
      toDOM: (m) => ["span", { class: "cmt", "data-ids": m.attrs.ids }, 0],
    },
  },
});

export function isList(n: PMNode): boolean {
  return n.type === schema.nodes.para && (n.attrs.kind === "bullet" || n.attrs.kind === "number");
}
