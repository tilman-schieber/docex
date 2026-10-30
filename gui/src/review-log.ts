// What you decided while reviewing this document (this session): for the
// progress shown in the review panel and the "done reviewing" summary.

import type { Item, ItemKind } from "./analyze";

export type Action = "accepted" | "rejected" | "resolved" | "reopened" | "deleted" | "replied";

export interface Decided {
  action: Action;
  author: string;
  kind: ItemKind;
  old?: string;
  new?: string;
  /** Comment text or the commented passage. */
  text?: string;
  section: string;
}

interface Entry extends Decided {
  /** Decisions made together (one backend call) are undone together. */
  group: number;
  /** Replies can't be taken back with Undo. */
  undoable: boolean;
}

let entries: Entry[] = [];
let group = 0;
let file: string | null = null;

/** Start over for another document. */
export function resetFor(path: string | null) {
  if (path === file) return;
  file = path;
  entries = [];
}

function push(action: Action, items: Item[], section: (it: Item) => string, g: number) {
  for (const it of items) {
    entries.push({
      action,
      author: it.author,
      kind: it.kind,
      old: it.old,
      new: it.new,
      text: it.kind === "comment" ? it.comment?.anchor || it.text : undefined,
      section: section(it),
      group: g,
      undoable: action !== "replied",
    });
  }
}

/** Record one backend call's decisions. */
export function record(action: Action, items: Item[], section: (it: Item) => string) {
  push(action, items, section, ++group);
}

/** Several kinds of decision made in one call (triage). */
export function recordMany(parts: [Action, Item[]][], section: (it: Item) => string) {
  const g = ++group;
  for (const [action, items] of parts) push(action, items, section, g);
}

/** The last undoable call was undone. */
export function undoLast() {
  const last = [...entries].reverse().find((e) => e.undoable);
  if (last) entries = entries.filter((e) => e.group !== last.group);
}

export function decided(author?: string): Decided[] {
  return entries.filter((e) => (e.action === "accepted" || e.action === "rejected") && (author === undefined || e.author === author));
}

export function count(action: Action): number {
  return entries.filter((e) => e.action === action).length;
}

const quote = (s?: string) => `"${(s ?? "").trim().replace(/\s+/g, " ").slice(0, 60)}"`;

function what(d: Decided): string {
  if (d.kind === "replace") return `${quote(d.old)} → ${quote(d.new)}`;
  if (d.kind === "insert") return `added ${quote(d.new)}`;
  if (d.kind === "delete") return `removed ${quote(d.old)}`;
  return d.kind === "format" ? "formatting" : "a change";
}

/** Plain text to paste into a reply: decisions per person, what was rejected, what is still open. */
export function summaryText(opts: { file: string; me: string; open: Item[]; remaining: number; mine: number }): string {
  const lines: string[] = [];
  const date = new Date().toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
  lines.push(`Review of ${opts.file} (${opts.me}, ${date})`, "");
  const all = decided();
  const acc = all.filter((d) => d.action === "accepted").length;
  const rej = all.length - acc;
  lines.push(`I went through ${all.length} of ${all.length + opts.remaining} changes: ${acc} accepted, ${rej} rejected${opts.remaining ? `, ${opts.remaining} still open` : ""}.`);
  const people = [...new Set(all.map((d) => d.author))];
  for (const p of people) {
    const mine = all.filter((d) => d.author === p);
    const a = mine.filter((d) => d.action === "accepted").length;
    const r = mine.filter((d) => d.action === "rejected");
    lines.push("", `${p}: ${a} accepted${r.length ? `, ${r.length} rejected` : ""}`);
    // Rejections in detail; the same edit made several times once, with a count.
    const groups = new Map<string, { d: Decided; n: number; sections: Set<string> }>();
    for (const d of r) {
      const k = what(d);
      const g = groups.get(k) ?? { d, n: 0, sections: new Set() };
      g.n++;
      if (d.section) g.sections.add(d.section);
      groups.set(k, g);
    }
    for (const [k, g] of groups) lines.push(`  - not taken: ${k}${g.n > 1 ? ` ×${g.n}` : ""}${g.sections.size ? ` (${[...g.sections].join(", ")})` : ""}`);
  }
  const resolved = count("resolved");
  const replied = count("replied");
  const deleted = count("deleted");
  if (resolved || replied || deleted) {
    lines.push("", `Comments: ${[resolved && `${resolved} resolved`, replied && `${replied} answered`, deleted && `${deleted} deleted`].filter(Boolean).join(", ")}.`);
  }
  if (opts.open.length) {
    lines.push("", `Still open (${opts.open.length}):`);
    for (const t of opts.open) lines.push(`  - ${t.author}${t.comment?.anchor ? ` on ${quote(t.comment.anchor)}` : ""}: ${(t.comment?.text ?? "").trim()}`);
  }
  if (opts.mine) lines.push("", `My own edits: ${opts.mine} tracked change${opts.mine === 1 ? "" : "s"}.`);
  return lines.join("\n");
}
