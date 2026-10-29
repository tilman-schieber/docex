// DocModel (from the docex core) <-> ProseMirror document.

import { Fragment, type Mark, type Node as PMNode } from "prosemirror-model";
import { schema } from "./schema";
import type { DocModel, MBlock, MPara, Span } from "./types";

const { nodes, marks } = schema;

function spanMarks(s: Span): Mark[] {
  const out: Mark[] = [];
  if (s.b) out.push(marks.strong.create());
  if (s.i) out.push(marks.em.create());
  if (s.s) out.push(marks.strike.create());
  if (s.c) out.push(marks.code.create());
  if (s.link) out.push(marks.link.create({ href: s.link }));
  if (s.st === "ins" || s.st === "del") {
    out.push(marks[s.st].create({ author: s.author || "", date: s.date || "", wid: s.wid || "" }));
  }
  if (s.comments && s.comments.length) out.push(marks.comment.create({ ids: s.comments.join(" ") }));
  return out;
}

function inlineNodes(spans: Span[]): PMNode[] {
  const out: PMNode[] = [];
  for (const s of spans) {
    const m = spanMarks(s);
    if (s.obj !== undefined) {
      const o = s.obj;
      if (o === "\n") out.push(nodes.hard_break.create(null, null, m));
      else if (o === "\t") out.push(schema.text("\t", m));
      else if (o.startsWith("img:")) out.push(nodes.image.create({ src: o.slice(4), alt: s.alt || "" }, null, m));
      else if (o.startsWith("fn:")) out.push(nodes.fnref.create({ label: o.slice(3) }, null, m));
      else out.push(nodes.obj.create({ key: o }, null, m));
    } else if (s.t) {
      out.push(schema.text(s.t, m));
    }
  }
  return out;
}

function para(p: MPara): PMNode {
  return nodes.para.create({ kind: p.kind || "normal", level: p.level || 0 }, inlineNodes(p.spans));
}

function paras(ps: MPara[]): PMNode[] {
  return ps.length ? ps.map(para) : [nodes.para.create()];
}

function block(b: MBlock): PMNode {
  switch (b.type) {
    case "para":
      return para(b);
    case "table":
      return nodes.table.create(
        null,
        b.rows.map((r) => nodes.table_row.create(null, r.map((c) => nodes.table_cell.create(null, paras(c))))),
      );
    case "opaque":
      return nodes.opaque.create({ id: b.id, label: b.label, preview: b.preview });
  }
}

export function modelToDoc(m: DocModel): PMNode {
  const out: PMNode[] = [];
  const boxesAt = (i: number | null) => m.boxes.filter((b) => b.after === i).map((b) => nodes.textbox.create({ id: b.id }, paras(b.paras)));
  out.push(...boxesAt(null));
  m.blocks.forEach((b, i) => {
    out.push(block(b));
    out.push(...boxesAt(i));
  });
  for (const n of m.notes) out.push(nodes.footnote.create({ label: n.label }, paras(n.paras)));
  if (!out.length) out.push(nodes.para.create());
  return nodes.doc.create(null, Fragment.from(out));
}

/** A comment to create: on characters [start, end) of the paragraph at `pos`. */
export interface PendingComment {
  pos: number;
  start: number;
  end: number;
  text: string;
}

function inlineSpans(p: PMNode): Span[] {
  const out: Span[] = [];
  p.forEach((child) => {
    const s: Span = {};
    for (const m of child.marks) {
      if (m.type === marks.strong) s.b = true;
      else if (m.type === marks.em) s.i = true;
      else if (m.type === marks.strike) s.s = true;
      else if (m.type === marks.code) s.c = true;
      else if (m.type === marks.link) s.link = m.attrs.href;
      else if (m.type === marks.ins) s.st = "ins";
      else if (m.type === marks.del) s.st = "del";
    }
    if (child.isText) s.t = child.text;
    else if (child.type === nodes.hard_break) s.obj = "\n";
    else if (child.type === nodes.image) {
      s.obj = "img:" + child.attrs.src;
      if (child.attrs.alt) s.alt = child.attrs.alt;
    } else if (child.type === nodes.fnref) s.obj = "fn:" + child.attrs.label;
    else if (child.type === nodes.obj) s.obj = child.attrs.key;
    else return;
    out.push(s);
  });
  return out;
}

function paraModel(p: PMNode, pos: number, pending?: PendingComment): MPara {
  const m: MPara = { kind: p.attrs.kind, level: p.attrs.level, spans: inlineSpans(p) };
  if (pending && pending.pos === pos) m.new_comments = [{ start: pending.start, end: pending.end, text: pending.text }];
  return m;
}

function parasIn(n: PMNode, pos: number, pending?: PendingComment): MPara[] {
  const out: MPara[] = [];
  n.forEach((child, off) => {
    if (child.type === nodes.para) out.push(paraModel(child, pos + 1 + off, pending));
  });
  return out;
}

export function docToModel(doc: PMNode, pending?: PendingComment): DocModel {
  const m: DocModel = { blocks: [], boxes: [], notes: [] };
  doc.forEach((n, pos) => {
    if (n.type === nodes.para) m.blocks.push({ type: "para", ...paraModel(n, pos, pending) });
    else if (n.type === nodes.table) {
      const rows: MPara[][][] = [];
      n.forEach((row, rOff) => {
        const cells: MPara[][] = [];
        row.forEach((cell, cOff) => cells.push(parasIn(cell, pos + 1 + rOff + 1 + cOff, pending)));
        rows.push(cells);
      });
      m.blocks.push({ type: "table", rows });
    } else if (n.type === nodes.opaque) {
      m.blocks.push({ type: "opaque", id: n.attrs.id, label: n.attrs.label, preview: n.attrs.preview });
    } else if (n.type === nodes.textbox) {
      m.boxes.push({ id: n.attrs.id, after: m.blocks.length ? m.blocks.length - 1 : null, paras: parasIn(n, pos, pending) });
    } else if (n.type === nodes.footnote) {
      m.notes.push({ label: n.attrs.label, paras: parasIn(n, pos, pending) });
    }
  });
  return m;
}

/** Index in docex atoms (code points; inline objects count one) of `offset` within paragraph `p`. */
export function atomIndex(p: PMNode, offset: number): number {
  let n = 0;
  let seen = 0;
  p.forEach((child) => {
    if (seen >= offset) return;
    const take = Math.min(child.nodeSize, offset - seen);
    n += child.isText ? [...child.text!.slice(0, take)].length : 1;
    seen += child.nodeSize;
  });
  return n;
}
