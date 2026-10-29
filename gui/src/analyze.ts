// Index the document for navigation and review: sections (headings), review
// items (tracked changes and comment threads) placed in those sections, and
// per-person statistics.

import type { Node as PMNode } from "prosemirror-model";
import { schema } from "./schema";
import type { Change, Comment, View } from "./types";

export interface Section {
  idx: number;
  /** 0 = text before the first heading, 1..6 headings, 1 for "Footnotes". */
  level: number;
  title: string;
  pos: number;
  parent: number | null;
  changes: number;
  comments: number;
  /** Review items per author, for the activity bar. */
  byAuthor: Map<string, number>;
}

export type ItemKind = "insert" | "delete" | "replace" | "format" | "other" | "comment";

export interface Item {
  key: string;
  kind: ItemKind;
  author: string;
  date: string;
  /** Word ids to accept / reject (empty for pending edits and comments). */
  wids: string[];
  /** Document range, if the item is visible in the text. */
  from: number | null;
  to: number | null;
  section: number;
  old?: string;
  new?: string;
  text?: string;
  before?: string;
  after?: string;
  /** Your own edits not yet sent to the core. */
  pending?: boolean;
  /** The marked ranges making up the item. */
  parts: { from: number; to: number; type: "ins" | "del" }[];
  label?: string;
  comment?: Comment;
  replies?: Comment[];
  /** Low-signal change: formatting only, whitespace only, punctuation only. */
  noise?: "formatting" | "whitespace" | "punctuation";
  /** Key shared by identical edits made in several places. */
  cluster?: string;
}

/** The same edit made in several places (e.g. "KI" -> "AI" 37 times). */
export interface Cluster {
  key: string;
  kind: ItemKind;
  old: string;
  new: string;
  items: Item[];
  authors: string[];
}

export interface Person {
  name: string;
  inserts: number;
  deletes: number;
  other: number;
  comments: number;
  replies: number;
  last: string;
}

export interface Index {
  sections: Section[];
  items: Item[];
  people: Person[];
  clusters: Cluster[];
  totals: { changes: number; comments: number; open: number; changePeople: number; commentPeople: number; pending: number; noise: number };
}

const PUNCT = /^[\s\p{P}\p{S}]*$/u;

function classify(it: Item): Item["noise"] {
  if (it.kind === "comment") return undefined;
  if (it.kind === "format") return "formatting";
  const o = it.old ?? "";
  const n = it.new ?? "";
  if (!o && !n) return undefined;
  if (o.replace(/\s+/g, "") === n.replace(/\s+/g, "")) return "whitespace";
  if (PUNCT.test(o) && PUNCT.test(n) && (o + n).trim().length <= 3) return "punctuation";
  return undefined;
}

interface Range {
  type: "ins" | "del";
  wid: string;
  author: string;
  date: string;
  from: number;
  to: number;
  text: string;
}

const CONTEXT = 36;

function leafText(n: PMNode): string {
  if (n.type === schema.nodes.hard_break) return " ";
  if (n.type === schema.nodes.image) return "[image]";
  if (n.type === schema.nodes.fnref) return `[${n.attrs.label}]`;
  if (n.type === schema.nodes.math) return n.attrs.display ? `$$${n.attrs.latex}$$` : `$${n.attrs.latex}$`;
  return "";
}

/** Displayed text of the paragraph around [from, to), without other deletions. */
function context(doc: PMNode, from: number, to: number): { before: string; after: string } {
  const $f = doc.resolve(from);
  const start = $f.start();
  const end = $f.end();
  const visible = (a: number, b: number) => {
    let s = "";
    doc.nodesBetween(a, b, (n, pos) => {
      if (n.isText) {
        if (schema.marks.del.isInSet(n.marks)) return false;
        s += n.text!.slice(Math.max(0, a - pos), Math.max(0, b - pos));
      } else if (n.isLeaf) s += leafText(n);
      return true;
    });
    return s;
  };
  let before = visible(start, from);
  let after = visible(Math.min(to, end), end);
  if (before.length > CONTEXT) before = "…" + before.slice(before.length - CONTEXT).replace(/^\S*\s/, "");
  if (after.length > CONTEXT) after = after.slice(0, CONTEXT).replace(/\s\S*$/, "") + "…";
  return { before, after };
}

const KIND_OF: Record<string, ItemKind> = {
  insert: "insert",
  "move-to": "insert",
  "paragraph-insert": "insert",
  "row-insert": "insert",
  delete: "delete",
  "move-from": "delete",
  "paragraph-delete": "delete",
  "row-delete": "delete",
  format: "format",
  "paragraph-format": "format",
  "table-format": "format",
  "section-format": "format",
};

const LABEL: Record<string, string> = {
  "paragraph-insert": "new paragraph",
  "paragraph-delete": "removed paragraph",
  "row-insert": "new table row",
  "row-delete": "removed table row",
  "move-to": "moved here",
  "move-from": "moved away",
  format: "formatting",
  "paragraph-format": "paragraph style",
  "table-format": "table layout",
  "section-format": "page setup",
};

export function analyze(doc: PMNode, v: View | null, me: string): Index {
  // Sections from headings (and the footnote area).
  const sections: Section[] = [];
  const mk = (level: number, title: string, pos: number): Section => {
    let parent: number | null = null;
    for (let i = sections.length - 1; i >= 0; i--) {
      if (sections[i].level > 0 && sections[i].level < level) {
        parent = i;
        break;
      }
    }
    return { idx: sections.length, level, title, pos, parent, changes: 0, comments: 0, byAuthor: new Map() };
  };
  sections.push(mk(0, "Beginning", 0));
  let inNotes = false;
  doc.forEach((n, pos) => {
    if (n.type === schema.nodes.para && /^h[1-6]$/.test(n.attrs.kind)) {
      sections.push(mk(Number(n.attrs.kind[1]), n.textContent.trim() || "(untitled)", pos));
      inNotes = false;
    } else if (n.type === schema.nodes.footnote && !inNotes) {
      sections.push(mk(1, "Footnotes", pos));
      inNotes = true;
    }
  });
  const starts = sections.map((s) => s.pos);
  const sectionAt = (pos: number) => {
    let lo = 0;
    let hi = starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (starts[mid] <= pos) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  };

  // Tracked-change ranges and comment anchors in the text.
  const ranges: Range[] = [];
  const commentPos = new Map<string, { from: number; to: number; text: string }>();
  doc.descendants((n, pos) => {
    if (!n.isInline) return true;
    const text = n.isText ? n.text! : leafText(n);
    for (const m of n.marks) {
      if (m.type === schema.marks.ins || m.type === schema.marks.del) {
        const type = m.type === schema.marks.ins ? "ins" : "del";
        const wid: string = m.attrs.wid || "";
        const last = ranges[ranges.length - 1];
        if (last && last.type === type && last.wid === wid && last.author === m.attrs.author && last.to === pos) {
          last.to = pos + n.nodeSize;
          last.text += text;
        } else {
          ranges.push({ type, wid, author: m.attrs.author || me, date: m.attrs.date || "", from: pos, to: pos + n.nodeSize, text });
        }
      } else if (m.type === schema.marks.comment) {
        for (const id of String(m.attrs.ids).split(" ").filter(Boolean)) {
          const c = commentPos.get(id);
          if (!c) commentPos.set(id, { from: pos, to: pos + n.nodeSize, text });
          else if (c.to === pos) {
            c.to = pos + n.nodeSize;
            c.text += text;
          }
        }
      }
    }
    return false;
  });

  // Change items: the core's grouped changes, placed via their Word ids.
  const byWid = new Map<string, Range[]>();
  for (const r of ranges) {
    if (!r.wid) continue;
    const list = byWid.get(r.wid);
    if (list) list.push(r);
    else byWid.set(r.wid, [r]);
  }
  let items: Item[] = [];
  const changes: Change[] = v?.changes ?? [];
  for (const c of changes) {
    const rs = c.wids.flatMap((w) => byWid.get(w) ?? []).sort((a, b) => a.from - b.from);
    const kind = KIND_OF[c.kind] ?? "other";
    const it: Item = {
      key: `c${c.n}`,
      kind,
      author: c.author || "unknown",
      date: c.date,
      wids: c.wids,
      from: rs.length ? rs[0].from : null,
      to: rs.length ? rs[rs.length - 1].to : null,
      section: -1,
      label: LABEL[c.kind],
      parts: rs.map((r) => ({ from: r.from, to: r.to, type: r.type })),
    };
    const ins = rs.filter((r) => r.type === "ins").map((r) => r.text).join("");
    const del = rs.filter((r) => r.type === "del").map((r) => r.text).join("");
    if (ins) it.new = ins;
    if (del) it.old = del;
    if (!ins && !del) it.text = c.text && c.text !== "¶" ? c.text : "";
    items.push(it);
  }
  // Pending edits (no Word id yet): contiguous ranges.
  let pend = 0;
  for (const r of ranges) {
    if (r.wid) continue;
    items.push({
      key: `p${pend++}`,
      kind: r.type === "ins" ? "insert" : "delete",
      author: me,
      date: "",
      wids: [],
      from: r.from,
      to: r.to,
      section: -1,
      pending: true,
      parts: [{ from: r.from, to: r.to, type: r.type }],
      ...(r.type === "ins" ? { new: r.text } : { old: r.text }),
    });
  }

  // Merge a deletion directly followed by an insertion (same person) into a replacement.
  items.sort((a, b) => (a.from ?? Infinity) - (b.from ?? Infinity));
  const merged: Item[] = [];
  for (const it of items) {
    const prev = merged[merged.length - 1];
    if (
      prev &&
      prev.kind === "delete" &&
      it.kind === "insert" &&
      prev.author === it.author &&
      !!prev.pending === !!it.pending &&
      prev.to !== null &&
      it.from !== null &&
      it.from - prev.to <= 1 &&
      !prev.label &&
      !it.label
    ) {
      prev.kind = "replace";
      prev.new = it.new;
      prev.to = it.to;
      prev.wids = [...prev.wids, ...it.wids];
      prev.parts = [...prev.parts, ...it.parts];
      prev.key += "+" + it.key;
      continue;
    }
    merged.push(it);
  }
  items = merged;

  // Comment threads.
  const comments = v?.comments ?? [];
  for (const c of comments.filter((c) => !c.parent)) {
    const at = commentPos.get(c.id);
    items.push({
      key: `m${c.id}`,
      kind: "comment",
      author: c.author || "unknown",
      date: c.date,
      wids: [],
      from: at?.from ?? null,
      to: at?.to ?? null,
      section: -1,
      parts: [],
      text: c.text,
      comment: c,
      replies: comments.filter((r) => r.parent === c.id),
    });
  }

  for (const it of items) {
    if (it.from !== null) {
      it.section = sectionAt(it.from);
      if (it.kind !== "comment") Object.assign(it, context(doc, it.from, it.to ?? it.from));
    }
  }
  items.sort((a, b) => (a.from ?? Infinity) - (b.from ?? Infinity));

  for (const it of items) {
    const s = sections[it.section];
    if (!s) continue;
    if (it.kind === "comment") s.comments++;
    else s.changes++;
    s.byAuthor.set(it.author, (s.byAuthor.get(it.author) ?? 0) + 1);
  }
  // People.
  const people = new Map<string, Person>();
  const person = (name: string) => {
    let p = people.get(name);
    if (!p) people.set(name, (p = { name, inserts: 0, deletes: 0, other: 0, comments: 0, replies: 0, last: "" }));
    return p;
  };
  for (const it of items) {
    if (it.kind === "comment") continue;
    const p = person(it.author);
    if (it.kind === "insert") p.inserts++;
    else if (it.kind === "delete") p.deletes++;
    else if (it.kind === "replace") {
      p.inserts++;
      p.deletes++;
    } else p.other++;
    if (it.date > p.last) p.last = it.date;
  }
  for (const c of comments) {
    const p = person(c.author || "unknown");
    if (c.parent) p.replies++;
    else p.comments++;
    if (c.date > p.last) p.last = c.date;
  }
  // Noise and repeated edits.
  const groups = new Map<string, Item[]>();
  for (const it of items) {
    it.noise = classify(it);
    if (it.kind === "comment" || it.pending || it.kind === "format" || it.label) continue;
    const o = (it.old ?? "").trim();
    const n = (it.new ?? "").trim();
    if (!o && !n) continue;
    const k = `${it.kind}\u0001${o}\u0001${n}`;
    const g = groups.get(k);
    if (g) g.push(it);
    else groups.set(k, [it]);
  }
  const clusters: Cluster[] = [];
  for (const [k, g] of groups) {
    if (g.length < 2) continue;
    for (const it of g) it.cluster = k;
    clusters.push({ key: k, kind: g[0].kind, old: (g[0].old ?? "").trim(), new: (g[0].new ?? "").trim(), items: g, authors: [...new Set(g.map((i) => i.author))] });
  }
  clusters.sort((a, b) => b.items.length - a.items.length);

  const changeItems = items.filter((i) => i.kind !== "comment");
  const threads = items.filter((i) => i.kind === "comment");
  return {
    sections,
    items,
    clusters,
    people: [...people.values()].sort((a, b) => b.inserts + b.deletes + b.other + b.comments - (a.inserts + a.deletes + a.other + a.comments)),
    totals: {
      changes: changeItems.length,
      comments: threads.length,
      open: threads.filter((t) => !t.comment?.done).length,
      changePeople: new Set(changeItems.map((i) => i.author)).size,
      commentPeople: new Set(comments.map((c) => c.author)).size,
      pending: changeItems.filter((i) => i.pending).length,
      noise: changeItems.filter((i) => i.noise).length,
    },
  };
}

/** Items in a section and its subsections. */
export function inSubtree(sections: Section[], root: number, s: number): boolean {
  for (let cur: number | null = s; cur !== null; cur = sections[cur]?.parent ?? null) {
    if (cur === root) return true;
  }
  return false;
}
