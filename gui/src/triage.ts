// Triage: review changes and comment threads one at a time with single keys.
// Decisions are collected locally (instant, undoable) and applied together
// in one call at the end.

import type { Item } from "./analyze";
import { visible } from "./cockpit";
import { authorColor, initials } from "./theme";

type Decision = "accept" | "reject" | "skip" | "resolve";

export interface TriageHost {
  /** Scroll the document to the item and highlight it; `inset` px at the top are covered by the triage panel. */
  show(it: Item, inset: number): void;
  /** Apply all decisions at once. */
  apply(decisions: { accept: Item[]; reject: Item[]; resolve: Item[] }, label: string): Promise<unknown>;
  reply(it: Item, text: string): Promise<unknown>;
  ask(title: string): Promise<string | null>;
  /** Section path of the item, for the card header. */
  section(it: Item): string;
  done(): void;
}

function h<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", ...kids: (Node | string | null | false)[]) {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  for (const k of kids) if (k) el.append(k);
  return el;
}

const KIND: Record<string, string> = { insert: "inserted", delete: "deleted", replace: "replaced", format: "formatting", other: "changed", comment: "comment" };

export function openTriage(items: Item[], scope: string, host: TriageHost) {
  if (!items.length) return;
  const decisions = new Map<string, Decision>();
  const history: string[] = [];
  let i = 0;
  let summary = false;
  const root = h("div", "triage");
  root.tabIndex = -1;
  document.getElementById("page-area")!.append(root);

  const count = (d: Decision) => [...decisions.values()].filter((x) => x === d).length;

  function close() {
    root.remove();
    document.removeEventListener("keydown", onKey, true);
    host.done();
  }

  async function apply() {
    const pick = (d: Decision) => items.filter((it) => decisions.get(it.key) === d);
    const chosen = { accept: pick("accept"), reject: pick("reject"), resolve: pick("resolve").filter((it) => it.comment) };
    const label = `Triage: ${[count("accept") && `${count("accept")} accepted`, count("reject") && `${count("reject")} rejected`, count("resolve") && `${count("resolve")} resolved`].filter(Boolean).join(", ")}`;
    close();
    if (chosen.accept.length || chosen.reject.length || chosen.resolve.length) await host.apply(chosen, label);
  }

  function decide(d: Decision) {
    const it = items[i];
    if (it.kind === "comment" && (d === "accept" || d === "reject")) return;
    if (it.kind !== "comment" && d === "resolve") return;
    decisions.set(it.key, d);
    history.push(it.key);
    next();
  }

  function next() {
    if (i < items.length - 1) i++;
    else summary = true;
    render();
  }

  function prev() {
    if (summary) summary = false;
    else if (i > 0) i--;
    render();
  }

  function undo() {
    const k = history.pop();
    if (!k) return;
    decisions.delete(k);
    i = items.findIndex((x) => x.key === k);
    summary = false;
    render();
  }

  async function reply() {
    const it = items[i];
    const text = await host.ask(it.kind === "comment" ? `Reply to ${it.author}` : "Comment on this change");
    root.focus();
    if (text?.trim()) await host.reply(it, text.trim());
  }

  function card(it: Item) {
    const d = decisions.get(it.key);
    const head = h(
      "div",
      "tr-head",
      h("span", "tr-sec", it.section >= 0 ? `§ ${host.section(it)}` : "Not in the text"),
    );
    const av = h("span", "av", initials(it.author));
    av.style.setProperty("--c", authorColor(it.author));
    const meta = h("div", "tr-meta", av, h("b", "", it.author), ` · ${it.label ?? KIND[it.kind]}`, it.date ? ` · ${it.date.slice(0, 10)}` : "", it.noise ? h("span", "tr-noise", it.noise) : null, it.cluster ? h("span", "tr-noise", "repeated edit") : null);
    const body = h("div", "tr-body");
    if (it.kind === "comment") {
      const c = it.comment!;
      if (c.anchor) body.append(h("div", "tr-quote", c.anchor));
      body.append(h("div", "tr-text", c.text));
      for (const r of it.replies ?? []) body.append(h("div", "tr-reply", h("b", "", r.author), ": ", r.text));
    } else {
      const line = h("div", "tr-text");
      if (it.before) line.append(h("span", "ctx", it.before));
      const ctx = !!(it.before || it.after);
      if (it.old) line.append(h("del", "", visible(it.old, ctx)));
      if (it.kind === "replace") line.append(h("span", "arrow", " → "));
      if (it.new) line.append(h("ins", "", visible(it.new, ctx)));
      if (!it.old && !it.new) line.append(h("span", "plain", it.text || it.label || "change"));
      if (it.after) line.append(h("span", "ctx", it.after));
      body.append(line);
    }
    const state = d ? h("div", `tr-state ${d}`, { accept: "✓ accepted", reject: "✗ rejected", skip: "skipped", resolve: "✓ resolved" }[d]) : null;
    return h("div", "tr-card", head, meta, body, state);
  }

  function render() {
    const done = decisions.size;
    const bar = h("div", "tr-progress");
    const fill = h("div", "tr-fill");
    fill.style.width = `${(done / items.length) * 100}%`;
    bar.append(fill);
    const top = h(
      "div",
      "tr-top",
      h("span", "tr-title", `Triage · ${scope}`),
      h("span", "tr-count", summary ? `${items.length} items` : `${i + 1} / ${items.length}`),
      h("span", "tr-tally", `${count("accept")} ✓  ${count("reject")} ✗  ${count("resolve")} ◉  ${count("skip")} skipped`),
      h("span", "grow"),
      h("button", "ghost", "Finish (Enter)"),
      h("button", "", "Cancel (Esc)"),
    );
    const [finish, cancel] = top.querySelectorAll("button");
    finish.addEventListener("click", () => (summary ? void apply() : ((summary = true), render())));
    cancel.addEventListener("click", close);
    const it = items[i];
    let main: HTMLElement;
    if (summary) {
      main = h(
        "div",
        "tr-card tr-summary",
        h("div", "tr-text", `Apply ${count("accept")} accepted and ${count("reject")} rejected changes${count("resolve") ? ` and resolve ${count("resolve")} comment threads` : ""}?`),
        h("div", "tr-meta", `${items.length - decisions.size} undecided and ${count("skip")} skipped items stay as they are.`),
      );
      const go = h("button", "primary", "Apply (Enter)");
      go.addEventListener("click", () => void apply());
      const back = h("button", "ghost", "Back (←)");
      back.addEventListener("click", prev);
      main.append(h("div", "row", back, go));
    } else main = card(it);
    const keys = h(
      "div",
      "tr-keys",
      ...(it?.kind === "comment" && !summary
        ? [["x", "resolve"], ["c", "reply"], ["s / →", "skip"], ["←", "back"], ["u", "undo"]]
        : [["a", "accept"], ["r", "reject"], ["s / →", "skip"], ["c", "comment"], ["←", "back"], ["u", "undo"]]
      ).map(([k, l]) => h("span", "", h("kbd", "", k), ` ${l}`)),
    );
    root.replaceChildren(top, bar, main, summary ? "" : keys);
    root.focus();
    // The panel covers the top of the page; the item shows in the text below it.
    if (!summary) host.show(it, root.offsetHeight);
  }

  function onKey(e: KeyboardEvent) {
    if (document.getElementById("modal") && !document.getElementById("modal")!.hidden) return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const k = e.key.toLowerCase();
    const act: Record<string, () => void> = summary
      ? { enter: () => void apply(), arrowleft: prev, escape: close, u: undo }
      : {
          a: () => decide("accept"),
          r: () => decide("reject"),
          x: () => decide("resolve"),
          s: () => decide("skip"),
          arrowright: next,
          j: next,
          arrowleft: prev,
          k: prev,
          u: undo,
          c: () => void reply(),
          enter: () => {
            summary = true;
            render();
          },
          escape: close,
        };
    const f = act[k];
    if (f) {
      e.preventDefault();
      e.stopPropagation();
      f();
    }
  }
  document.addEventListener("keydown", onKey, true);
  render();
}
