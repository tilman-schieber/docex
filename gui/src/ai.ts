// AI assistance. Rewrites come back as tracked changes in your name (only the
// words that changed), replies as editable drafts; nothing is applied silently.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Mark, Node as PMNode } from "prosemirror-model";
import type { EditorState, Transaction } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import type { Index, Item } from "./analyze";
import { diffWords } from "./diff";
import { schema } from "./schema";

export interface AIHost {
  view: EditorView;
  author(): string;
  toast(msg: string, kind?: "info" | "error"): void;
  index(): Index;
}

interface SettingsView {
  provider: string;
  anthropic_model: string;
  effort: string;
  openai_base_url: string;
  openai_model: string;
  anthropic_key_set: boolean;
  anthropic_key_from_env: boolean;
  openai_key_set: boolean;
}

let counter = 0;

/** Run a request; `onText` receives the text so far as it streams in. */
export async function runAI(system: string, prompt: string, onText?: (soFar: string) => void, maxTokens?: number): Promise<string> {
  const id = `ai${Date.now()}-${counter++}`;
  let soFar = "";
  const un = await listen<{ id: string; text: string }>("ai-delta", (e) => {
    if (e.payload.id !== id) return;
    soFar += e.payload.text;
    onText?.(soFar);
  });
  try {
    return await invoke<string>("ai_complete", { req: { id, system, prompt, max_tokens: maxTokens } });
  } finally {
    un();
  }
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Record<string, unknown> = {}, ...kids: (Node | string | null)[]) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "class") e.className = String(v);
    else (e as unknown as Record<string, unknown>)[k] = v;
  }
  for (const k of kids) if (k !== null) e.append(k);
  return e;
}

// ---------------------------------------------------------------------------
// Settings

export async function aiSettingsDialog(): Promise<void> {
  const s = await invoke<SettingsView>("ai_settings");
  return new Promise((resolve) => {
    const modal = document.getElementById("modal")!;
    const provider = el("select", {}, el("option", { value: "anthropic" }, "Anthropic (Claude)"), el("option", { value: "openai" }, "OpenAI-compatible endpoint")) as HTMLSelectElement;
    provider.value = s.provider;
    const aKey = el("input", { type: "password", placeholder: s.anthropic_key_from_env ? "using ANTHROPIC_API_KEY" : s.anthropic_key_set ? "•••••• (stored; type to replace)" : "sk-ant-…" }) as HTMLInputElement;
    const aModel = el("input", { value: s.anthropic_model }) as HTMLInputElement;
    const effort = el("select", {}, ...["low", "medium", "high"].map((x) => el("option", { value: x }, x))) as HTMLSelectElement;
    effort.value = s.effort;
    const oUrl = el("input", { value: s.openai_base_url, placeholder: "https://…/v1" }) as HTMLInputElement;
    const oKey = el("input", { type: "password", placeholder: s.openai_key_set ? "•••••• (stored; type to replace)" : "optional" }) as HTMLInputElement;
    const oModel = el("input", { value: s.openai_model, placeholder: "model name" }) as HTMLInputElement;
    const field = (label: string, input: HTMLElement) => el("label", { class: "field" }, el("span", {}, label), input);
    const aBox = el(
      "div",
      { class: "fields" },
      field("API key", aKey),
      field("Model", aModel),
      field("Effort", effort),
      el("div", { class: "hint" }, "Claude Max includes monthly API credits ($100 on Max 5x, $200 on Max 20x): claim them on claude.ai → Settings → Billing → API credits, then create a key at platform.claude.com. Keys are stored in ~/.config/docex/ai.json (readable only by you)."),
    );
    const oBox = el("div", { class: "fields" }, field("Base URL", oUrl), field("API key", oKey), field("Model", oModel), el("div", { class: "hint" }, "Any /chat/completions endpoint: OpenAI, OpenRouter, Ollama, vLLM, LM Studio, …"));
    const sync = () => {
      aBox.hidden = provider.value !== "anthropic";
      oBox.hidden = provider.value !== "openai";
    };
    provider.addEventListener("change", sync);
    sync();
    const close = () => {
      modal.hidden = true;
      modal.replaceChildren();
      resolve();
    };
    const save = async () => {
      try {
        await invoke("ai_settings_set", {
          update: {
            provider: provider.value,
            anthropic_model: aModel.value,
            effort: effort.value,
            openai_base_url: oUrl.value,
            openai_model: oModel.value,
            anthropic_key: aKey.value || null,
            openai_key: oKey.value || null,
          },
        });
        close();
      } catch (e) {
        alertIn(String(e));
      }
    };
    const status = el("div", { class: "hint err" });
    const alertIn = (m: string) => (status.textContent = m);
    modal.replaceChildren(
      el(
        "div",
        { class: "dialog wide" },
        el("label", {}, "AI settings"),
        field("Provider", provider),
        aBox,
        oBox,
        status,
        el("div", { class: "row" }, el("button", { onclick: close }, "Cancel"), el("button", { class: "primary", onclick: () => void save() }, "Save")),
      ),
    );
    modal.hidden = false;
  });
}

// ---------------------------------------------------------------------------
// Rewriting a selection as tracked changes

interface Ch {
  ch: string;
  pos: number;
  marks: readonly Mark[];
}

/** Selected paragraphs as editable characters, or an error message. */
export function selectionParas(state: EditorState): { paras: Ch[][] } | string {
  const { from, to, empty, $from } = state.selection;
  let a = from;
  let b = to;
  if (empty) {
    if ($from.parent.type !== schema.nodes.para) return "Put the cursor in a paragraph or select some text.";
    a = $from.start();
    b = $from.end();
  }
  const paras: Ch[][] = [];
  let problem = "";
  state.doc.nodesBetween(a, b, (node, pos) => {
    if (node.type !== schema.nodes.para) return true;
    const chars: Ch[] = [];
    node.forEach((child, off) => {
      const start = pos + 1 + off;
      if (start + child.nodeSize <= a || start >= b) return;
      if (schema.marks.del.isInSet(child.marks)) return;
      if (!child.isText) {
        problem = "The selection contains images, formulas or other objects; select plain text.";
        return;
      }
      for (let k = 0; k < child.text!.length; k++) {
        const p = start + k;
        if (p >= a && p < b) chars.push({ ch: child.text![k], pos: p, marks: child.marks });
      }
    });
    if (chars.some((c) => c.ch.trim())) paras.push(chars);
    return false;
  });
  if (problem) return problem;
  if (!paras.length) return "Nothing to rewrite.";
  if (paras.reduce((n, p) => n + p.length, 0) > 20000) return "Selection too long; rewrite a section at a time.";
  return { paras };
}

/** Turn the new text of each paragraph into minimal tracked changes. */
export function rewriteTransaction(state: EditorState, paras: Ch[][], newTexts: string[], author: string): { tr: Transaction; edits: number } {
  const { ins, del, comment } = schema.marks;
  const insMark = ins.create({ author, date: "", wid: "" });
  const delMark = del.create({ author, date: "", wid: "" });
  const tr = state.tr.setMeta("docex-raw", true);
  type Edit = { kind: "mark"; from: number; to: number } | { kind: "drop"; from: number; to: number } | { kind: "ins"; at: number; text: string; marks: readonly Mark[] };
  const edits: Edit[] = [];
  paras.forEach((chars, pi) => {
    const old = chars.map((c) => c.ch).join("");
    let k = 0;
    for (const op of diffWords(old, newTexts[pi])) {
      if (op.type === "eq") {
        k += op.text.length;
      } else if (op.type === "del") {
        for (let j = k; j < k + op.text.length; j++) {
          const c = chars[j];
          const own = ins.isInSet(c.marks);
          // Your own not-yet-saved insertion simply goes away.
          const kind = own && (!own.attrs.wid || own.attrs.author === author) ? "drop" : "mark";
          const last = edits[edits.length - 1];
          if (last && last.kind === kind && "to" in last && last.to === c.pos) last.to = c.pos + 1;
          else edits.push({ kind, from: c.pos, to: c.pos + 1 } as Edit);
        }
        k += op.text.length;
      } else {
        const at = k < chars.length ? chars[k].pos : chars[chars.length - 1].pos + 1;
        const near = chars[Math.max(0, k - 1)] ?? chars[0];
        const marks = near.marks.filter((m) => m.type !== ins && m.type !== del && m.type !== comment);
        edits.push({ kind: "ins", at, text: op.text, marks });
      }
    }
  });
  for (const e of edits) if (e.kind === "mark") tr.addMark(e.from, e.to, delMark);
  const moves = edits.filter((e) => e.kind !== "mark").sort((x, y) => ("at" in y ? y.at : y.from) - ("at" in x ? x.at : x.from));
  for (const e of moves) {
    if (e.kind === "drop") tr.delete(e.from, e.to);
    else if (e.kind === "ins") tr.insert(e.at, schema.text(e.text, [...e.marks, insMark]));
  }
  return { tr: tr.scrollIntoView(), edits: edits.length };
}

export const REWRITES: [string, string][] = [
  ["Tighten", "Make it more concise. Cut filler and redundancy; keep all facts."],
  ["Fix grammar & spelling", "Fix grammar, spelling and punctuation only. Change nothing else."],
  ["More formal", "Make the tone more formal and professional."],
  ["Simpler", "Make it simpler and easier to read, with shorter sentences."],
  ["Clearer", "Make it clearer and more precise without changing its meaning."],
  ["Translate to English", "Translate it to English."],
  ["Translate to German", "Translate it to German."],
];

function statusBox(title: string) {
  const box = el("div", { class: "ai-status" }, el("div", { class: "ai-status-title" }, title), el("div", { class: "ai-status-text" }));
  document.body.append(box);
  return {
    text: (t: string) => ((box.lastChild as HTMLElement).textContent = t.slice(-600)),
    close: () => box.remove(),
  };
}

export async function rewriteSelection(host: AIHost, instruction: string, label = instruction) {
  const view = host.view;
  const sel = selectionParas(view.state);
  if (typeof sel === "string") return host.toast(sel);
  const docBefore = view.state.doc;
  const texts = sel.paras.map((p) => p.map((c) => c.ch).join(""));
  const st = statusBox(`AI · ${label}…`);
  try {
    const out = await runAI(
      "You edit passages of a document for its author. Apply the instruction to the passage and return only the rewritten passage: no preamble, no quotes, no comments, no Markdown formatting. Keep exactly the same number of paragraphs, separated by one blank line. Keep the passage's language unless the instruction asks for a translation. Keep names, numbers and facts.",
      `Instruction: ${instruction}\n\nPassage:\n${texts.join("\n\n")}`,
      st.text,
    );
    let parts = out.trim().split(/\n\s*\n/).map((p) => p.replace(/\s*\n\s*/g, " ").trim());
    if (parts.length !== texts.length) {
      if (texts.length === 1) parts = [parts.join(" ")];
      else return host.toast(`The AI returned ${parts.length} paragraphs for ${texts.length}; nothing changed. Try one paragraph at a time.`, "error");
    }
    if (view.state.doc !== docBefore) return host.toast("The document changed while the AI was working; nothing applied.", "error");
    const { tr, edits: n } = rewriteTransaction(view.state, sel.paras, parts, host.author());
    view.dispatch(tr);
    host.toast(n ? `${label}: suggestions added as tracked changes in your name.` : `${label}: no changes suggested.`);
  } catch (e) {
    host.toast(String(e), "error");
  } finally {
    st.close();
  }
}

// ---------------------------------------------------------------------------
// The document as text, for questions and summaries

function inlineText(n: PMNode): string {
  let s = "";
  n.forEach((c) => {
    if (schema.marks.del.isInSet(c.marks)) return;
    if (c.isText) s += c.text;
    else if (c.type === schema.nodes.math) s += c.attrs.display ? `$$${c.attrs.latex}$$` : `$${c.attrs.latex}$`;
    else if (c.type === schema.nodes.fnref) s += `[^${c.attrs.label}]`;
    else if (c.type === schema.nodes.hard_break) s += "\n";
    else if (c.type === schema.nodes.image) s += "[image]";
  });
  return s;
}

export function docMarkdown(doc: PMNode): string {
  const out: string[] = [];
  const para = (p: PMNode) => {
    const t = inlineText(p).trim();
    if (!t) return;
    const k: string = p.attrs.kind;
    if (/^h[1-6]$/.test(k)) out.push(`${"#".repeat(Number(k[1]))} ${t}`);
    else if (k === "bullet") out.push(`${"  ".repeat(p.attrs.level)}- ${t}`);
    else if (k === "number") out.push(`${"  ".repeat(p.attrs.level)}1. ${t}`);
    else if (k === "quote") out.push(`> ${t}`);
    else out.push(t);
  };
  doc.forEach((n) => {
    if (n.type === schema.nodes.para) para(n);
    else if (n.type === schema.nodes.table) {
      n.forEach((row) => {
        const cells: string[] = [];
        row.forEach((cell) => {
          const t: string[] = [];
          cell.forEach((p) => t.push(inlineText(p).trim()));
          cells.push(t.filter(Boolean).join(" / "));
        });
        out.push(`| ${cells.join(" | ")} |`);
      });
    } else if (n.type === schema.nodes.textbox) n.forEach(para);
    else if (n.type === schema.nodes.footnote) out.push(`[^${n.attrs.label}]: ${n.textContent}`);
    else if (n.type === schema.nodes.opaque) out.push(`[${n.attrs.label}]`);
  });
  return out.join("\n\n");
}

function reviewList(index: Index, items: Item[]): string {
  const sec = (i: Item) => (i.section >= 0 ? index.sections[i.section].title : "-");
  return items
    .map((i) => {
      if (i.kind === "comment") return `- [comment by ${i.author}, §${sec(i)}${i.comment?.done ? ", resolved" : ""}] on "${i.comment?.anchor ?? ""}": ${i.text}${(i.replies ?? []).map((r) => ` | reply by ${r.author}: ${r.text}`).join("")}`;
      const what = i.kind === "replace" ? `"${i.old}" → "${i.new}"` : i.kind === "insert" ? `inserted "${i.new ?? i.text ?? ""}"` : i.kind === "delete" ? `deleted "${i.old ?? i.text ?? ""}"` : (i.label ?? i.kind);
      return `- [${i.author}, §${sec(i)}] ${what}`;
    })
    .join("\n");
}

// ---------------------------------------------------------------------------
// Answer panel

export function answerPanel(host: AIHost, title: string, run: (onText: (t: string) => void) => Promise<string>) {
  const modal = document.getElementById("modal")!;
  const out = el("div", { class: "ai-out" }, "…");
  const copy = el("button", { onclick: () => void navigator.clipboard.writeText(out.textContent ?? "") }, "Copy");
  const close = () => {
    modal.hidden = true;
    modal.replaceChildren();
    host.view.focus();
  };
  modal.replaceChildren(el("div", { class: "dialog wide" }, el("label", {}, title), out, el("div", { class: "row" }, copy, el("button", { class: "primary", onclick: close }, "Close"))));
  modal.hidden = false;
  run((t) => (out.textContent = t))
    .then((t) => (out.textContent = t))
    .catch((e) => {
      out.textContent = String(e);
      out.classList.add("err");
    });
}

export async function askDocument(host: AIHost, question: string) {
  const doc = docMarkdown(host.view.state.doc);
  answerPanel(host, `AI · ${question}`, (on) =>
    runAI(
      "You answer questions about a document for its reader. Be concise and specific; quote the document where it helps; say so when the document doesn't say. Answer in the language of the question.",
      `<document>\n${doc}\n</document>\n\nQuestion: ${question}`,
      on,
    ),
  );
}

export async function summarizeReview(host: AIHost, items: Item[], scope: string) {
  const index = host.index();
  if (!items.length) return host.toast("No changes or comments to summarize.");
  answerPanel(host, `AI · Summary of ${scope}`, (on) =>
    runAI(
      "You summarize the review state of a document for its author: who changed what and why it matters, grouped by person and theme, then the open questions from comments. Lead with what needs the author's decision. Be concise; plain text with short bullet points. Answer in the language of the document.",
      `<document>\n${docMarkdown(host.view.state.doc).slice(0, 120000)}\n</document>\n\n<changes_and_comments>\n${reviewList(index, items)}\n</changes_and_comments>`,
      on,
    ),
  );
}

/** Draft a reply to a comment thread, streamed into `onText`. */
export async function draftReply(host: AIHost, it: Item, onText: (t: string) => void): Promise<string> {
  const c = it.comment!;
  const index = host.index();
  const sec = it.section >= 0 ? index.sections[it.section].title : "";
  return runAI(
    `You draft a reply, for ${host.author()}, to a comment on their document. Reply in the language of the comment, briefly and constructively: agree and say what changes, or explain politely why not. Return only the reply text.`,
    `<document>\n${docMarkdown(host.view.state.doc).slice(0, 120000)}\n</document>\n\nSection: ${sec}\nCommented text: "${c.anchor}"\nComment by ${c.author}: ${c.text}\n${(it.replies ?? []).map((r) => `Reply by ${r.author}: ${r.text}`).join("\n")}`,
    onText,
    2000,
  );
}
