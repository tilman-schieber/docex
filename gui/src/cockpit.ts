// The review cockpit: document outline, review feed with people and filters,
// and the activity rail next to the page.

import {
  ListChecks,
  Sparkles,
  Check,
  ChevronDown,
  ChevronRight,
  createElement,
  CornerDownRight,
  MessageSquare,
  Search,
  Trash2,
  Undo2,
  X,
} from "lucide";
import { TextSelection } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import { analyze, inSubtree, type Index, type Item, type Person } from "./analyze";
import { schema } from "./schema";
import { setHighlight } from "./highlight";
import { authorColor, initials } from "./theme";
import type { View } from "./types";

export interface App {
  view: EditorView;
  current(): View | null;
  me(): string;
  call(cmd: string, args?: Record<string, unknown>): Promise<View | undefined>;
  toast(msg: string, kind?: "info" | "error"): void;
  /** Whether Track Changes is on. */
  tracking(): boolean;
  /** Review the given items one by one. */
  triage(items: Item[], scope: string): void;
  /** AI summary of the given items. */
  summarize(items: Item[], scope: string): void;
  /** AI-drafted reply to a comment thread. */
  draft(it: Item, onText: (t: string) => void): Promise<string>;
  /** The "since you last saw it" chip, if there is a baseline. */
  sinceChip(): HTMLElement | null;
}

type IconNode = Parameters<typeof createElement>[0];

const $ = <T extends HTMLElement>(sel: string) => document.querySelector(sel) as T;

function h<K extends keyof HTMLElementTagNameMap>(tag: K, props: Record<string, unknown> = {}, ...kids: (Node | string | null | undefined | false)[]) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "class") el.className = String(v);
    else if (k === "style" && typeof v === "string") el.setAttribute("style", v);
    else if (k.startsWith("data-")) el.setAttribute(k, String(v));
    else (el as unknown as Record<string, unknown>)[k] = v;
  }
  for (const k of kids) if (k !== null && k !== undefined && k !== false) el.append(k);
  return el;
}

function icon(node: IconNode, cls = "ico"): SVGElement {
  const el = createElement(node);
  el.setAttribute("class", cls);
  return el;
}

function ago(d: string): string {
  if (!d) return "";
  const t = Date.parse(d);
  if (Number.isNaN(t)) return d;
  const s = (Date.now() - t) / 1000;
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  if (s < 86400 * 14) return `${Math.round(s / 86400)}d ago`;
  return new Date(t).toLocaleDateString(undefined, { day: "numeric", month: "short", year: s > 86400 * 300 ? "numeric" : undefined });
}

/** Whitespace, or punctuation without surrounding text, is hard to see: name it or quote it. */
export function visible(t: string, inContext = false): string {
  if (!t || /[\p{L}\p{N}]/u.test(t)) return t;
  if (!t.trim()) return t.includes("\n") ? "¶" : t.length === 1 ? "space" : `${t.length} spaces`;
  return inContext ? t : `“${t.trim()}”`;
}

function plural(n: number, one: string, many = one + "s") {
  return `${n} ${n === 1 ? one : many}`;
}

function avatar(name: string, small = false) {
  return h("span", { class: `av${small ? " sm" : ""}`, style: `--c:${authorColor(name)}`, title: name }, initials(name));
}

export function createCockpit(app: App) {
  const view = app.view;
  const wrap = $("#page-wrap");
  let index: Index = analyze(view.state.doc, null, app.me());
  const filter = {
    people: new Set<string>(),
    show: "all" as "all" | "changes" | "comments" | "open",
    q: "",
    section: null as number | null,
    hideNoise: false,
    cluster: null as string | null,
  };
  let active: string | null = null;
  let expanded: string | null = null;
  /** Move the caret into the expanded thread's reply box (only when opened from the list). */
  let focusReply = false;
  /** Thread whose delete button was clicked once and waits for confirmation. */
  let confirmDelete: string | null = null;
  let hlTimer: number | undefined;
  const collapsed = new Set<string>(JSON.parse(localStorage.getItem("docex-collapsed") || "[]"));
  let headingTops: number[] = [];
  let currentSection = 0;

  // ------------------------------------------------------------------------
  // Filtering

  function matches(it: Item, ignore: "people" | "section" | null = null) {
    if (ignore !== "people" && filter.people.size && !filter.people.has(it.author)) return false;
    if (filter.show === "changes" && it.kind === "comment") return false;
    if ((filter.show === "comments" || filter.show === "open") && it.kind !== "comment") return false;
    if (filter.show === "open" && it.comment?.done) return false;
    if (ignore !== "section" && filter.section !== null && !inSubtree(index.sections, filter.section, it.section)) return false;
    if (filter.hideNoise && it.noise) return false;
    if (filter.cluster !== null && it.cluster !== filter.cluster) return false;
    if (filter.q) {
      const q = filter.q.toLowerCase();
      const hay = [it.old, it.new, it.text, it.author, it.label, ...(it.replies ?? []).map((r) => r.text)].join(" ").toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  }

  const shown = () => index.items.filter((i) => matches(i));

  // ------------------------------------------------------------------------
  // Navigation

  function absTop(pos: number): number {
    try {
      const c = view.coordsAtPos(Math.min(pos, view.state.doc.content.size));
      return c.top - wrap.getBoundingClientRect().top + wrap.scrollTop;
    } catch {
      return 0;
    }
  }

  function highlight(it: Item | null, on: boolean) {
    if (!it || !on) return setHighlight(view, null);
    const ranges = it.parts.length ? it.parts : it.from !== null ? [{ from: it.from, to: it.to ?? it.from }] : [];
    setHighlight(view, ranges.length ? ranges : null);
  }

  /** `hold` keeps the highlight until the next one; `inset` is page height covered at the top (triage). */
  function focusItem(it: Item, scroll = true, opts: { hold?: boolean; inset?: number } = {}) {
    active = it.key;
    document.querySelectorAll(".item.active").forEach((e) => e.classList.remove("active"));
    const row = document.querySelector(`.item[data-key="${CSS.escape(it.key)}"]`);
    row?.classList.add("active");
    row?.scrollIntoView({ block: "nearest" });
    if (it.from !== null && scroll) {
      const inset = opts.inset ?? 0;
      wrap.scrollTo({ top: absTop(it.from) - inset - (wrap.clientHeight - inset) / 3 });
      const tr = view.state.tr.setSelection(TextSelection.create(view.state.doc, it.from));
      view.dispatch(tr.setMeta("addToHistory", false));
    }
    highlight(it, true);
    clearTimeout(hlTimer);
    if (!opts.hold) hlTimer = window.setTimeout(() => highlight(it, false), 1400);
  }

  function step(d: number) {
    const list = shown().filter((i) => i.from !== null);
    if (!list.length) return;
    let k = list.findIndex((i) => i.key === active);
    if (k < 0) {
      const here = view.state.selection.from;
      k = d > 0 ? list.findIndex((i) => i.from! > here) : list.length - list.slice().reverse().findIndex((i) => i.from! < here) - 1;
      if (k < 0 || k >= list.length) k = d > 0 ? 0 : list.length - 1;
    } else k = (k + d + list.length) % list.length;
    focusItem(list[k]);
  }

  function scrollToSection(i: number) {
    const s = index.sections[i];
    if (!s) return;
    wrap.scrollTo({ top: i === 0 ? 0 : absTop(s.pos + 1) - 16 });
    if (i > 0) view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, s.pos + 1)).setMeta("addToHistory", false));
  }

  // ------------------------------------------------------------------------
  // Accept / reject

  /** Accept or reject your own pending edits directly in the editor. */
  function resolveLocal(items: Item[], accept: boolean) {
    const parts = items.filter((i) => i.pending).flatMap((i) => i.parts).sort((a, b) => b.from - a.from);
    if (!parts.length) return;
    const tr = view.state.tr.setMeta("docex-raw", true);
    for (const p of parts) {
      const keepText = (p.type === "ins") === accept;
      if (keepText) tr.removeMark(p.from, p.to, p.type === "ins" ? schema.marks.ins : schema.marks.del);
      else tr.delete(p.from, p.to);
    }
    view.dispatch(tr);
  }

  const sig = (i: Item) => `${i.kind}\u0001${i.old ?? ""}\u0001${i.new ?? ""}`;

  async function resolve(items: Item[], accept: boolean) {
    refreshNow();
    const keys = new Set(items.map((i) => i.key));
    const fresh = index.items.filter((i) => keys.has(i.key));
    // Accepting your own unsaved insertion with Track Changes on: make it a
    // real tracked change first and accept that; just dropping the mark here
    // would have the next sync track it again.
    if (accept && app.tracking() && fresh.some((i) => i.pending && i.parts.some((p) => p.type === "ins"))) {
      const synced = fresh.filter((i) => !i.pending);
      const want = fresh.filter((i) => i.pending).map(sig);
      await app.call("sync");
      refreshNow();
      const mine = index.items.filter((i) => !i.pending && i.author === app.me() && i.kind !== "comment");
      const used = new Set<string>();
      const mapped: Item[] = [];
      for (const w of want) {
        const m = mine.find((i) => !used.has(i.key) && sig(i) === w);
        if (m) {
          used.add(m.key);
          mapped.push(m);
        }
      }
      const wids = [...synced, ...mapped].flatMap((i) => i.wids);
      if (wids.length) await app.call("resolve", { wids, accept });
      return;
    }
    resolveLocal(fresh, accept);
    const wids = fresh.filter((i) => !i.pending).flatMap((i) => i.wids);
    if (wids.length) await app.call("resolve", { wids, accept });
    else refreshNow();
  }

  // ------------------------------------------------------------------------
  // Outline

  function renderOutline() {
    const el = $("#outline");
    const secs = index.sections;
    const has = (i: number) => secs[i].level > 0 || index.items.some((it) => it.section === 0) || (secs.length === 1);
    const counts = secs.map(() => ({ changes: 0, comments: 0, by: new Map<string, number>() }));
    for (const it of index.items) {
      if (it.section < 0 || !matches(it, "section")) continue;
      // Count into the section and its ancestors (shown when collapsed).
      for (let s: number | null = it.section; s !== null; s = secs[s].parent) {
        const c = counts[s];
        if (it.kind === "comment") c.comments++;
        else c.changes++;
        c.by.set(it.author, (c.by.get(it.author) ?? 0) + 1);
        if (!collapsed.has(key(s))) break;
      }
    }
    const max = Math.max(1, ...counts.map((c) => c.changes + c.comments));
    const kids = (i: number) => secs.some((s) => s.parent === i);
    const hidden = (i: number) => {
      for (let p = secs[i].parent; p !== null; p = secs[p].parent) if (collapsed.has(key(p))) return true;
      return false;
    };
    const total = index.items.filter((i) => matches(i, "section")).length;
    const list = h("div", { class: "ol-list" });
    secs.forEach((s, i) => {
      if (!has(i) || hidden(i)) return;
      const c = counts[i];
      const n = c.changes + c.comments;
      const bar = h("span", { class: "ol-bar" });
      for (const [a, k] of c.by) bar.append(h("span", { style: `width:${(k / max) * 100}%;background:${authorColor(a)}`, title: `${a}: ${k}` }));
      const row = h(
        "div",
        {
          class: `ol-item lvl-${Math.min(s.level, 4)}${i === currentSection ? " current" : ""}${filter.section === i ? " filtered" : ""}${n ? "" : " quiet"}`,
          onclick: () => scrollToSection(i),
        },
        kids(i)
          ? h(
              "span",
              {
                class: "twisty",
                onclick: (e: Event) => {
                  e.stopPropagation();
                  const k = key(i);
                  if (collapsed.has(k)) collapsed.delete(k);
                  else collapsed.add(k);
                  localStorage.setItem("docex-collapsed", JSON.stringify([...collapsed]));
                  renderOutline();
                },
              },
              icon(collapsed.has(key(i)) ? ChevronRight : ChevronDown, "ico tiny"),
            )
          : h("span", { class: "twisty" }),
        h("span", { class: "ol-title", title: s.title }, s.title),
        n
          ? h(
              "span",
              {
                class: "ol-badges",
                "data-tip": "Changes / comments here (click: show only this section)",
                onclick: (e: Event) => {
                  e.stopPropagation();
                  filter.section = filter.section === i ? null : i;
                  renderAll();
                },
              },
              c.changes ? h("span", { class: "b-chg" }, String(c.changes)) : null,
              c.comments ? h("span", { class: "b-cmt" }, String(c.comments)) : null,
            )
          : null,
        bar,
      );
      list.append(row);
    });
    const words = view.state.doc.textContent.split(/\s+/).filter(Boolean).length;
    el.replaceChildren(h("div", { class: "pane-head" }, h("span", {}, "Outline"), h("span", { class: "muted" }, `${words.toLocaleString()} words`)), list);
    if (secs.length === 1 && !total) el.append(h("p", { class: "empty" }, "Headings will show up here."));
  }

  const key = (i: number) => `${index.sections[i].level}:${index.sections[i].title}`;

  function updateCurrentSection() {
    const y = wrap.scrollTop + 80;
    let cur = 0;
    for (let i = 1; i < headingTops.length; i++) if (headingTops[i] <= y) cur = i;
    if (cur !== currentSection) {
      currentSection = cur;
      document.querySelectorAll(".ol-item.current").forEach((e) => e.classList.remove("current"));
      renderOutline();
      document.querySelector(".ol-item.current")?.scrollIntoView({ block: "nearest" });
      onSection?.(index.sections[cur]?.title ?? "");
    }
  }

  // ------------------------------------------------------------------------
  // Review feed

  const hover = h("div", { id: "hovercard", hidden: true });
  document.body.append(hover);

  function personCard(p: Person, anchor: HTMLElement) {
    const total = p.inserts + p.deletes + p.other;
    const seg = (n: number, color: string, label: string) => (n ? h("span", { style: `flex:${n};background:${color}`, title: `${n} ${label}` }) : null);
    hover.replaceChildren(
      h("div", { class: "hc-head" }, avatar(p.name), h("div", {}, h("b", {}, p.name), h("div", { class: "muted" }, p.last ? `active ${ago(p.last)}` : ""))),
      h("div", { class: "hc-bar" }, seg(p.inserts, "var(--green)", "insertions"), seg(p.deletes, "var(--red)", "deletions"), seg(p.other, "var(--blue)", "other"), seg(p.comments + p.replies, "var(--yellow)", "comments")),
      h(
        "div",
        { class: "hc-stats" },
        h("span", {}, h("b", { style: "color:var(--green)" }, String(p.inserts)), " insertions"),
        h("span", {}, h("b", { style: "color:var(--red)" }, String(p.deletes)), " deletions"),
        p.other ? h("span", {}, h("b", { style: "color:var(--blue)" }, String(p.other)), " formatting / other") : null,
        h("span", {}, h("b", { style: "color:var(--yellow)" }, String(p.comments)), ` comments, ${plural(p.replies, "reply", "replies")}`),
      ),
      h("div", { class: "muted hc-hint" }, total + p.comments ? "click: only this person · shift+click: add" : ""),
    );
    const r = anchor.getBoundingClientRect();
    hover.hidden = false;
    hover.style.top = `${r.bottom + 6}px`;
    hover.style.left = `${Math.min(r.left, window.innerWidth - 300)}px`;
  }

  function changeLine(it: Item) {
    const line = h("div", { class: "line" });
    const ctx = !!(it.before || it.after);
    if (it.before) line.append(h("span", { class: "ctx" }, it.before));
    if (it.old) line.append(h("del", {}, visible(it.old, ctx)));
    if (it.kind === "replace") line.append(h("span", { class: "arrow" }, "→"));
    if (it.new) line.append(h("ins", {}, visible(it.new, ctx)));
    if (!it.old && !it.new) line.append(h("span", { class: "plain" }, it.text || it.label || "change"));
    if (it.after) line.append(h("span", { class: "ctx" }, it.after));
    return line;
  }

  const KIND_WORD: Record<string, string> = { insert: "inserted", delete: "deleted", replace: "replaced", format: "formatting", other: "changed" };

  function itemRow(it: Item) {
    const row = h("div", {
      class: `item ${it.kind}${it.pending ? " pending" : ""}${it.comment?.done ? " done" : ""}${active === it.key ? " active" : ""}`,
      "data-key": it.key,
      onmouseenter: () => highlight(it, true),
      onmouseleave: () => highlight(it, false),
      onclick: () => {
        if (it.kind === "comment") {
          expanded = expanded === it.key ? null : it.key;
          focusReply = true;
        }
        focusItem(it);
        if (it.kind === "comment") renderReview();
      },
    });
    const acts = h("div", { class: "acts" });
    const act = (ic: IconNode, tip: string, fn: () => unknown, cls = "") => {
      const b = h("button", { class: `act ${cls}`, onclick: (e: Event) => { e.stopPropagation(); void fn(); } });
      b.dataset.tip = tip;
      b.append(icon(ic));
      return b;
    };
    if (it.kind === "comment") {
      const c = it.comment!;
      acts.append(
        act(c.done ? Undo2 : Check, c.done ? "Reopen" : "Resolve", () => app.call("comment_done", { id: c.id, done: !c.done }), "ok"),
        confirmDelete === c.id
          ? h(
              "button",
              {
                class: "act no confirm",
                "data-tip": "Delete this thread and its replies",
                onclick: (e: Event) => {
                  e.stopPropagation();
                  confirmDelete = null;
                  void app.call("comment_delete", { id: c.id });
                },
              },
              icon(Trash2),
              "Delete?",
            )
          : act(Trash2, "Delete thread", () => {
              confirmDelete = c.id;
              renderFeed();
              setTimeout(() => {
                if (confirmDelete !== c.id) return;
                confirmDelete = null;
                renderFeed();
              }, 3000);
            }, "no"),
      );
      const body = h(
        "div",
        { class: "body" },
        c.anchor ? h("div", { class: "quote" }, c.anchor) : null,
        h("div", { class: "ctext" }, c.text),
        h(
          "div",
          { class: "meta" },
          h("span", { style: `color:${authorColor(it.author)}` }, it.author),
          ` · ${ago(it.date)}`,
          it.replies?.length ? h("span", { class: "replies" }, " · ", icon(MessageSquare, "ico tiny inline"), ` ${it.replies.length}`) : null,
          c.done ? " · resolved" : null,
        ),
      );
      if (expanded === it.key) {
        for (const r of it.replies ?? []) {
          body.append(h("div", { class: "reply" }, h("div", { class: "meta" }, avatar(r.author, true), h("span", { style: `color:${authorColor(r.author)}` }, r.author), ` · ${ago(r.date)}`), h("div", { class: "ctext" }, r.text)));
        }
        const ta = h("textarea", { rows: 2, placeholder: "Reply…  (Ctrl+Enter)", onclick: (e: Event) => e.stopPropagation() }) as HTMLTextAreaElement;
        const send = async () => {
          const text = ta.value.trim();
          if (text) await app.call("comment_reply", { id: c.id, text });
        };
        ta.onkeydown = (e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void send();
          }
          e.stopPropagation();
        };
        const sendBtn = h("button", { class: "ghost", onclick: (e: Event) => { e.stopPropagation(); void send(); } }, icon(CornerDownRight), "Reply");
        const draftBtn = h(
          "button",
          {
            class: "ghost",
            "data-tip": "Let AI draft a reply (you review it before sending)",
            onclick: async (e: Event) => {
              e.stopPropagation();
              draftBtn.disabled = true;
              try {
                ta.value = (await app.draft(it, (t) => (ta.value = t))).trim();
              } catch (err) {
                app.toast(String(err), "error");
              } finally {
                draftBtn.disabled = false;
                ta.focus();
              }
            },
          },
          icon(Sparkles),
          "Draft",
        ) as HTMLButtonElement;
        body.append(h("div", { class: "reply-box" }, ta, h("div", { class: "reply-btns" }, sendBtn, draftBtn)));
        if (focusReply) setTimeout(() => ta.focus(), 0);
        focusReply = false;
      }
      row.append(avatar(it.author), body, acts);
    } else {
      acts.append(act(Check, "Accept  Alt+A", () => resolve([it], true), "ok"), act(X, "Reject  Alt+R", () => resolve([it], false), "no"));
      row.append(
        avatar(it.author),
        h(
          "div",
          { class: "body" },
          changeLine(it),
          h(
            "div",
            { class: "meta" },
            h("span", { style: `color:${authorColor(it.author)}` }, it.author),
            ` · ${it.label ?? KIND_WORD[it.kind]}`,
            it.pending ? h("span", { class: "pend" }, " · not saved yet") : ` · ${ago(it.date)}`,
          ),
        ),
        acts,
      );
    }
    return row;
  }

  function renderReview() {
    const el = $("#sidebar");
    const t = index.totals;
    const list = shown();
    // Summary sentence.
    const summary = h(
      "div",
      { class: "summary" },
      t.changes || t.comments
        ? h(
            "span",
            {},
            h("b", {}, String(t.changes)),
            ` ${t.changes === 1 ? "change" : "changes"} by `,
            h("b", {}, String(t.changePeople)),
            ` · `,
            h("b", {}, String(t.comments)),
            ` ${t.comments === 1 ? "thread" : "threads"} (`,
            h("b", { style: "color:var(--yellow)" }, String(t.open)),
            ` open) by `,
            h("b", {}, String(t.commentPeople)),
            t.pending ? h("span", { class: "pend" }, ` · ${t.pending} unsaved`) : null,
          )
        : h("span", { class: "muted" }, "No changes or comments. Edit away."),
    );
    const extras = h("div", { class: "extras" });
    if (t.noise) {
      // Noise within the current filter (ignoring the hide-noise switch itself).
      const hidden = filter.hideNoise;
      filter.hideNoise = false;
      const noise = index.items.filter((i) => i.noise && matches(i));
      filter.hideNoise = hidden;
      extras.append(
        h(
          "button",
          {
            class: `tag${filter.hideNoise ? " on" : ""}`,
            "data-tip": "Formatting-only, whitespace and punctuation changes (click: hide them)",
            onclick: () => {
              filter.hideNoise = !filter.hideNoise;
              renderAll();
            },
          },
          filter.hideNoise ? `${t.noise} noise hidden` : `${t.noise} noise`,
        ),
        h(
          "button",
          { class: "tag ok", "data-tip": "Accept all formatting, whitespace and punctuation changes", onclick: () => resolve(noise, true) },
          icon(Check, "ico tiny"),
          "accept noise",
        ),
      );
    }
    if (index.clusters.length) extras.append(h("span", { class: "tag plain" }, `${index.clusters.length} repeated edits`));
    const since = app.sinceChip();
    if (since) extras.append(since);
    // People.
    const people = h("div", { class: "people" });
    for (const p of index.people) {
      const n = p.inserts + p.deletes + p.other + p.comments + p.replies;
      const chip = h(
        "button",
        {
          class: `chip${filter.people.has(p.name) ? " on" : ""}${filter.people.size && !filter.people.has(p.name) ? " off" : ""}`,
          style: `--c:${authorColor(p.name)}`,
          onclick: (e: MouseEvent) => {
            if (e.shiftKey) {
              if (filter.people.has(p.name)) filter.people.delete(p.name);
              else filter.people.add(p.name);
            } else if (filter.people.size === 1 && filter.people.has(p.name)) filter.people.clear();
            else {
              filter.people.clear();
              filter.people.add(p.name);
            }
            renderAll();
          },
          onmouseenter: (e: MouseEvent) => personCard(p, e.currentTarget as HTMLElement),
          onmouseleave: () => (hover.hidden = true),
        },
        avatar(p.name, true),
        h("span", { class: "nm" }, p.name.split(/\s+/)[0]),
        h("span", { class: "n" }, String(n)),
      );
      people.append(chip);
    }
    // Filters.
    const pill = (v: typeof filter.show, label: string, n: number) =>
      h("button", { class: `pill${filter.show === v ? " on" : ""}`, onclick: () => { filter.show = v; renderAll(); } }, label, h("span", { class: "n" }, String(n)));
    const base = index.items.filter((i) => (!filter.people.size || filter.people.has(i.author)) && (filter.section === null || inSubtree(index.sections, filter.section, i.section)));
    const search = h("input", {
      class: "search",
      placeholder: "Filter…",
      value: filter.q,
      oninput: (e: Event) => {
        filter.q = (e.target as HTMLInputElement).value;
        renderFeed();
        renderOutline();
        renderRail();
      },
    }) as HTMLInputElement;
    const filters = h(
      "div",
      { class: "filters" },
      pill("all", "All", base.length),
      pill("changes", "Changes", base.filter((i) => i.kind !== "comment").length),
      pill("comments", "Comments", base.filter((i) => i.kind === "comment").length),
      pill("open", "Open", base.filter((i) => i.kind === "comment" && !i.comment?.done).length),
      h("span", { class: "search-wrap" }, icon(Search, "ico tiny"), search),
    );
    // Active filters + bulk actions.
    const scope: string[] = [];
    if (filter.people.size) scope.push([...filter.people].map((p) => p.split(/\s+/)[0]).join(", "));
    if (filter.section !== null) scope.push(`§ ${index.sections[filter.section].title}`);
    if (filter.cluster !== null) {
      const c = index.clusters.find((x) => x.key === filter.cluster);
      if (c) scope.push(`"${c.old}" → "${c.new}"`);
    }
    const changeItems = list.filter((i) => i.kind !== "comment");
    const filtered = scope.length > 0 || !!filter.q || filter.show !== "all" || filter.hideNoise;
    // Unfiltered, the count is already on the "All" tab.
    const scopeRow = filtered
      ? h(
          "div",
          { class: "scope-row" },
          h("span", { class: "scope", title: scope.join(" · ") }, scope.length ? `Showing ${list.length} · ${scope.join(" · ")}` : `Showing ${list.length}`),
          h("button", { class: "clear", "data-tip": "Show everything again", onclick: () => { filter.people.clear(); filter.section = null; filter.q = ""; filter.show = "all"; filter.cluster = null; filter.hideNoise = false; renderAll(); } }, "clear"),
        )
      : null;
    const bulk = h(
      "div",
      { class: "bulk" },
      h("span", { class: "grow" }),
      list.length
        ? h("button", { class: "ghost", "data-tip": "Go through these one by one with single keys  Alt+T", onclick: () => startTriage() }, icon(ListChecks), "Triage")
        : null,
      list.length ? h("button", { class: "ghost", "data-tip": "AI summary of what's shown", onclick: () => app.summarize(list, scopeLabel()) }, icon(Sparkles)) : null,
      changeItems.length
        ? h("button", { class: "ghost ok", "data-tip": "Accept every change listed below", onclick: () => resolve(changeItems, true) }, icon(Check), `Accept ${changeItems.length}`)
        : null,
      changeItems.length
        ? h("button", { class: "ghost no", "data-tip": "Reject every change listed below", onclick: () => resolve(changeItems, false) }, icon(X), `Reject ${changeItems.length}`)
        : null,
    );
    const feed = h("div", { class: "feed", id: "feed" });
    el.replaceChildren(
      h("div", { class: "review-head" }, summary, extras.childNodes.length ? extras : "", people, filters, scopeRow, bulk),
      feed,
      h("div", { class: "keys muted" }, "Alt+↑↓ next/prev · Alt+A accept · Alt+R reject"),
    );
    renderFeed();
    if (document.activeElement?.classList.contains("search")) search.focus();
  }

  function renderFeed() {
    const feed = $("#feed");
    if (!feed) return;
    const list = shown();
    feed.replaceChildren();
    // Repeated edits first: accept or reject a pattern everywhere at once.
    const patterns = filter.cluster === null && filter.show !== "comments" && filter.show !== "open" ? index.clusters.filter((c) => c.items.some((i) => matches(i))) : [];
    if (patterns.length) {
      feed.append(h("div", { class: "sec-head" }, h("span", { class: "sec-title" }, "Repeated edits"), h("span", { class: "muted" }, String(patterns.length))));
      for (const c of patterns.slice(0, 12)) {
        const its = c.items.filter((i) => matches(i));
        const row = h(
          "div",
          { class: "item pattern", onclick: () => { filter.cluster = c.key; renderAll(); } },
          h("span", { class: "times" }, `×${its.length}`),
          h(
            "div",
            { class: "body" },
            h("div", { class: "line" }, c.old ? h("del", {}, visible(c.old)) : null, c.old && c.new ? h("span", { class: "arrow" }, "→") : null, c.new ? h("ins", {}, visible(c.new)) : null),
            h("div", { class: "meta" }, c.authors.join(", ")),
          ),
          h(
            "div",
            { class: "acts" },
            h("button", { class: "act ok", "data-tip": `Accept all ${its.length}`, onclick: (e: Event) => { e.stopPropagation(); void resolve(its, true); } }, icon(Check)),
            h("button", { class: "act no", "data-tip": `Reject all ${its.length}`, onclick: (e: Event) => { e.stopPropagation(); void resolve(its, false); } }, icon(X)),
          ),
        );
        feed.append(row);
      }
    }
    let sec = -2;
    for (const it of list) {
      if (it.section !== sec) {
        sec = it.section;
        const s = index.sections[sec];
        const inSec = list.filter((i) => i.section === sec);
        const secChanges = inSec.filter((i) => i.kind !== "comment");
        feed.append(
          h(
            "div",
            { class: "sec-head", onclick: () => (sec >= 0 ? scrollToSection(it.section) : undefined) },
            h("span", { class: "sec-title" }, s ? s.title : "Not in the text"),
            h("span", { class: "muted" }, String(inSec.length)),
            secChanges.length > 1
              ? h(
                  "span",
                  { class: "sec-acts" },
                  h("button", { "data-tip": "Accept all shown in this section", onclick: (e: Event) => { e.stopPropagation(); void resolve(secChanges, true); } }, icon(Check, "ico tiny")),
                  h("button", { "data-tip": "Reject all shown in this section", onclick: (e: Event) => { e.stopPropagation(); void resolve(secChanges, false); } }, icon(X, "ico tiny")),
                )
              : null,
          ),
        );
      }
      feed.append(itemRow(it));
    }
    if (!list.length) feed.append(h("p", { class: "empty" }, index.items.length ? "Nothing matches the filter." : "Tracked changes and comments will show up here."));
  }

  // ------------------------------------------------------------------------
  // Activity rail and author filter in the document

  function renderRail() {
    const rail = $("#rail");
    const H = wrap.scrollHeight || 1;
    const rh = rail.clientHeight || 1;
    const marks = h("div", { class: "rail-marks" });
    for (const it of index.items) {
      if (it.from === null) continue;
      const on = matches(it);
      const top = (absTop(it.from) / H) * rh;
      const m = h("div", {
        class: `rm ${it.kind === "comment" ? "c" : "x"}${on ? "" : " off"}`,
        style: `top:${top}px;--c:${it.kind === "comment" ? "var(--yellow)" : it.pending ? "var(--cyan)" : authorColor(it.author)}`,
        title: `${it.author}: ${it.kind === "comment" ? it.text : (it.old ?? "") + (it.new ? " → " + it.new : "")}`.slice(0, 120),
        onclick: (e: Event) => {
          e.stopPropagation();
          focusItem(it);
        },
      });
      marks.append(m);
    }
    const vp = h("div", { class: "rail-vp" });
    rail.replaceChildren(marks, vp);
    updateViewport();
  }

  function updateViewport() {
    const vp = document.querySelector(".rail-vp") as HTMLElement | null;
    if (!vp) return;
    const H = wrap.scrollHeight || 1;
    const rh = $("#rail").clientHeight;
    vp.style.top = `${(wrap.scrollTop / H) * rh}px`;
    vp.style.height = `${Math.max(12, (wrap.clientHeight / H) * rh)}px`;
  }

  $("#rail").addEventListener("click", (e) => {
    const r = $("#rail").getBoundingClientRect();
    wrap.scrollTo({ top: ((e.clientY - r.top) / r.height) * wrap.scrollHeight - wrap.clientHeight / 2 });
  });

  function applyDim() {
    let style = document.getElementById("dimstyle");
    if (!style) {
      style = h("style", { id: "dimstyle" });
      document.head.append(style);
    }
    if (!filter.people.size) {
      style.textContent = "";
      return;
    }
    const keep = [...filter.people].map((p) => CSS.escape(p));
    const ids = index.items.filter((i) => i.comment && filter.people.has(i.author)).map((i) => CSS.escape(i.comment!.id));
    style.textContent =
      `.ProseMirror ins.chg, .ProseMirror del.chg { opacity: .32; }\n` +
      `.ProseMirror .cmt { background: none; border-bottom-color: transparent; }\n` +
      keep.map((p) => `.ProseMirror ins[data-author="${p}"], .ProseMirror del[data-author="${p}"]`).join(",\n") +
      ` { opacity: 1; }\n` +
      (ids.length ? ids.map((id) => `.ProseMirror .cmt[data-ids~="${id}"]`).join(",\n") + ` { background: color-mix(in srgb, var(--yellow) 16%, transparent); border-bottom-color: color-mix(in srgb, var(--yellow) 70%, transparent); }` : "");
  }

  // ------------------------------------------------------------------------

  function measure() {
    headingTops = index.sections.map((s, i) => (i === 0 ? 0 : absTop(s.pos + 1)));
  }

  function renderAll() {
    renderOutline();
    renderReview();
    renderRail();
    applyDim();
  }

  function refreshNow() {
    index = analyze(view.state.doc, app.current(), app.me());
    if (filter.section !== null && filter.section >= index.sections.length) filter.section = null;
    measure();
    renderAll();
    updateCurrentSection();
  }

  let timer: number | undefined;
  function refreshSoon() {
    clearTimeout(timer);
    timer = window.setTimeout(refreshNow, 160);
  }

  let raf = 0;
  wrap.addEventListener("scroll", () => {
    if (raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      updateViewport();
      updateCurrentSection();
    });
  });
  window.addEventListener("resize", () => {
    measure();
    renderRail();
  });
  // Images change the layout once loaded.
  view.dom.addEventListener("load", () => refreshSoon(), true);

  document.addEventListener("keydown", (e) => {
    if (!e.altKey || e.ctrlKey || e.metaKey) return;
    const k = e.key.toLowerCase();
    if (k === "arrowdown" || k === "j") {
      e.preventDefault();
      step(1);
    } else if (k === "arrowup" || k === "k") {
      e.preventDefault();
      step(-1);
    } else if ((k === "a" || k === "r") && active) {
      const it = index.items.find((i) => i.key === active);
      if (!it || it.kind === "comment") return;
      e.preventDefault();
      const next = shown().filter((i) => i.from !== null && i.key !== it.key && (i.from ?? 0) >= (it.from ?? 0))[0];
      void resolve([it], k === "a").then(() => {
        const n = next && index.items.find((i) => i.from === next.from && i.kind === next.kind);
        if (n) focusItem(n);
      });
    }
  });
  document.addEventListener("mouseover", (e) => {
    if (!(e.target as HTMLElement).closest(".chip")) hover.hidden = true;
  });

  let onSection: ((title: string) => void) | undefined;

  function scopeLabel() {
    const parts: string[] = [];
    if (filter.people.size) parts.push([...filter.people].join(", "));
    if (filter.section !== null) parts.push(`§ ${index.sections[filter.section].title}`);
    if (filter.cluster !== null) {
      const c = index.clusters.find((x) => x.key === filter.cluster);
      if (c) parts.push(`"${c.old}" → "${c.new}"`);
    }
    if (filter.show !== "all") parts.push(filter.show);
    if (filter.hideNoise) parts.push("without noise");
    return parts.join(" · ") || "everything";
  }

  async function startTriage() {
    // Your unsaved edits need Word ids before they can be decided on.
    if (index.totals.pending) {
      await app.call("sync");
      refreshNow();
    }
    // Only items there is something to decide on: changes with Word ids, threads.
    const list = shown().filter((i) => !i.pending && (i.kind === "comment" || i.wids.length > 0));
    if (!list.length) return app.toast("Nothing to triage with the current filter.");
    app.triage(list, scopeLabel());
  }

  return {
    refreshNow,
    startTriage,
    index: () => index,
    filterTo(f: { person?: string; section?: number; cluster?: string; show?: typeof filter.show; hideNoise?: boolean }) {
      if (f.person !== undefined) {
        filter.people.clear();
        filter.people.add(f.person);
      }
      if (f.section !== undefined) filter.section = f.section;
      if (f.cluster !== undefined) filter.cluster = f.cluster;
      if (f.show !== undefined) filter.show = f.show;
      if (f.hideNoise !== undefined) filter.hideNoise = f.hideNoise;
      renderAll();
    },
    clearFilters() {
      filter.people.clear();
      filter.section = null;
      filter.q = "";
      filter.show = "all";
      filter.cluster = null;
      filter.hideNoise = false;
      renderAll();
    },
    shown,
    scrollToSection,
    focus: focusItem,
    resolve,
    rerender: () => renderReview(),
    refreshSoon,
    /** Open a thread in the list without taking the caret out of the document. */
    focusComment(id: string) {
      const it = index.items.find((i) => i.comment?.id === id);
      if (it) {
        expanded = it.key;
        focusReply = false;
        renderReview();
        focusItem(it, false);
      }
    },
    focusWid(wid: string) {
      const it = index.items.find((i) => i.wids.includes(wid));
      if (it) focusItem(it, false);
    },
    itemByWid: (wid: string) => index.items.find((i) => i.wids.includes(wid)) ?? null,
    changeLine,
    onSectionChange(fn: (title: string) => void) {
      onSection = fn;
    },
    stats() {
      return index.totals;
    },
  };
}
