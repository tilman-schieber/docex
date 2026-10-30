// AI assistance. Edits come back as tracked changes in your name, replies as
// editable drafts; nothing is applied silently.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Node as PMNode } from "prosemirror-model";
import { TextSelection, type EditorState } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import type { Index, Item } from "./analyze";
import { docToModel } from "./convert";
import { schema } from "./schema";
import type { DocModel } from "./types";

export interface AIHost {
  view: EditorView;
  author(): string;
  toast(msg: string, kind?: "info" | "error"): void;
  index(): Index;
  /** The document as docex Markdown, with the editor's content; `marked` locates the selection. */
  source(marked?: DocModel): Promise<Source>;
  /** Apply edited Markdown as tracked changes; `note` is added to the report. */
  applyMarkdown(md: string, label: string, note: string): Promise<void>;
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
// Editing the document as docex Markdown
//
// The AI sees what `docex cat` shows (formatting, links, headings, lists,
// tables, footnotes, other people's changes and comments) and answers with
// find/replace edits on it. The core applies the result like any edited
// Markdown, as tracked changes in your name: new links are real links, a
// paragraph that becomes a heading changes style, bolding is a format change.

/** The document's Markdown from the backend, and the part holding the selection. */
export interface Source {
  md: string;
  region: [number, number] | null;
}

export interface Edit {
  find: string;
  replace: string;
  /** Every occurrence, not just one. */
  all: boolean;
}

/** The selection marked with two private-use characters, for the backend to find it in the Markdown. */
export function markedModel(state: EditorState): DocModel {
  const { doc, selection } = state;
  // Inside text, also for a selection of table cells or a whole node.
  const a = TextSelection.near(doc.resolve(selection.from), 1).from;
  const b = Math.max(a, TextSelection.near(doc.resolve(selection.to), -1).to);
  return docToModel(state.tr.insertText("\uE001", b).insertText("\uE000", a).doc);
}

/** The edits in the AI's answer, and what it said besides (e.g. why nothing changed). */
export function parseEdits(out: string): { edits: Edit[]; note: string } {
  const edits: Edit[] = [];
  // One newline right inside a tag belongs to the layout, not to the text.
  const inner = (t: string) => t.replace(/^\r?\n/, "").replace(/\r?\n$/, "");
  const re = /<edit(\s+all="?true"?)?\s*>\s*<find>([\s\S]*?)<\/find>\s*<replace>([\s\S]*?)<\/replace>\s*<\/edit>/g;
  const note = out
    .replace(re, (_, all, find, replace) => {
      edits.push({ find: inner(find), replace: inner(replace), all: !!all });
      return "";
    })
    .replace(/<\/?none>/g, "")
    .trim();
  return { edits, note };
}

/** Apply edits to `md`, only within `region` when given. Each edit must match
 *  exactly once there (or anywhere, for `all`); the others are returned. */
export function applyEdits(md: string, edits: Edit[], region: [number, number] | null): { md: string; applied: number; failed: Edit[] } {
  const [lo, hi] = region ?? [0, md.length];
  const hits: { at: number; len: number; text: string }[] = [];
  const failed: Edit[] = [];
  for (const e of edits) {
    const found: number[] = [];
    if (e.find) for (let k = md.indexOf(e.find, lo); k >= 0 && k + e.find.length <= hi; k = md.indexOf(e.find, k + e.find.length)) found.push(k);
    if (!found.length || (found.length > 1 && !e.all)) failed.push(e);
    else for (const at of found) hits.push({ at, len: e.find.length, text: e.replace });
  }
  hits.sort((x, y) => x.at - y.at);
  // Overlapping edits: the first one wins.
  const keep = hits.filter((h, k) => !hits.slice(0, k).some((o) => o.at + o.len > h.at));
  let out = md;
  for (const h of [...keep].reverse()) out = out.slice(0, h.at) + h.text + out.slice(h.at + h.len);
  return { md: out, applied: keep.length, failed };
}

export const EDIT_SYSTEM = `You edit a Word document for its author. The document is shown as Markdown; your edits are written back into the Word file as tracked changes in the author's name, which they review.

How the Markdown maps to Word:
- \`#\` … \`######\` headings, \`- \` bullets, \`1. \` numbered items (indent two spaces per level), \`> \` quotes, \`: \` captions.
- **bold**, *italic*, ~~strikethrough~~, \`code\`, <u>underline</u>, <sup>raised</sup>, <sub>lowered</sub>, <mark>highlight</mark> (<mark data-color="green"> for other colours), <span style="color:#C00000">font colour</span>, [text](https://example.com) links.
- $…$ inline formulas and $$…$$ on their own line (LaTeX); they become Word equations.
- Footnotes: [^n] in the text, [^n]: text at the end. A new one: [^new] and [^new]: text.
- Tables: pipe tables, or HTML <table> with one <td> per line and colspan/rowspan for merged cells; <br> separates paragraphs in a cell.
- {++text++} and {--text--} are tracked changes by other people, {==text==}{>>Name: remark<<} are comments: keep them exactly as they are. To add a comment for the author, write {==passage==}{>>remark<<}.
- <!-- docex:… --> lines and ![](media/…) images stand for content that can't be edited here (table of contents, shapes, charts, headers and footers, text box and content control markers): never change or remove them.
- Keep escapes such as \\* \\_ \\$ as they are. Fonts, sizes and spacing can't be set.

Answer only with edits, each in this form:
<edit>
<find>exact text copied from the document</find>
<replace>the new version of that text</replace>
</edit>
The find text must occur exactly once in the part you may edit: include enough of the surrounding text to make it unique, but no more than needed. Use <edit all="true"> to change every occurrence. Edits must not overlap. Change only what the instruction asks for and leave everything else exactly as it is, including line breaks and blank lines. Keep the document's language unless asked to translate. If nothing should change, answer <none>why</none>.`;

/** The request for `aiEdit`: the document (or, when huge, the part around the selection), the selection, the instruction. */
export function editPrompt(md: string, region: [number, number] | null, instruction: string, picked = ""): string {
  if (!region) return `<document>\n${md}\n</document>\n\nInstruction: ${instruction}`;
  const pad = md.length > 120000 ? 20000 : md.length;
  const a = Math.max(0, region[0] - pad);
  const b = Math.min(md.length, region[1] + pad);
  const context = `${a > 0 ? "[…]\n\n" : ""}${md.slice(a, b)}${b < md.length ? "\n\n[…]" : ""}`;
  const part = md.slice(region[0], region[1]);
  const focus = picked && picked.length < part.length * 0.8 ? `Within it, the author selected: "${picked}"; focus on that.\n\n` : "";
  return `<document>\n${context}\n</document>\n\n<selection>\n${part}\n</selection>\n\n${focus}Edit only inside the selection. Instruction: ${instruction}`;
}

/** Edit the selection (the cursor's paragraph when empty) or, with `whole`, the whole document. */
export async function aiEdit(host: AIHost, instruction: string, label: string, whole = false) {
  const view = host.view;
  const docBefore = view.state.doc;
  const { from, to, empty } = view.state.selection;
  const picked = empty ? "" : docBefore.textBetween(from, to, " ");
  let src: Source;
  try {
    src = await host.source(whole ? undefined : markedModel(view.state));
  } catch (e) {
    return host.toast(String(e), "error");
  }
  const region = whole ? null : src.region;
  if (!whole && !region) return host.toast("Couldn't find the selection in the document; select some text and try again.", "error");
  if (whole && src.md.length > 400000) return host.toast("The document is too long to edit as a whole; select a part.", "error");
  const prompt = editPrompt(src.md, region, instruction, picked);
  const st = statusBox(`AI · ${label}…`);
  try {
    const out = await runAI(EDIT_SYSTEM, prompt, st.text, 32000);
    const { edits, note } = parseEdits(out);
    if (!edits.length) return host.toast(note ? `${label}: ${note.slice(0, 300)}` : `${label}: no changes suggested.`);
    if (view.state.doc !== docBefore) return host.toast("The document changed while the AI was working; nothing applied.", "error");
    const r = applyEdits(src.md, edits, region);
    if (!r.applied) return host.toast(`${label}: the AI's edits didn't match the document; nothing changed. Try again.`, "error");
    const skipped = r.failed.length ? ` ${r.failed.length} of ${edits.length} edits didn't match and were left out.` : "";
    await host.applyMarkdown(r.md, label, skipped);
  } catch (e) {
    host.toast(String(e), "error");
  } finally {
    st.close();
  }
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

/** What the AI suggests for one change in triage. */
export interface Advice {
  do: "accept" | "reject" | "check";
  why: string;
}

const ADVICE_MAX = 300;

/** The request for `reviewAdvice`: system prompt, prompt and token budget. */
export function advicePrompt(me: string, doc: string, changes: Item[], section: (i: Item) => string) {
  const q = (t?: string) => `«${(t ?? "").replace(/\s+/g, " ")}»`;
  const list = changes
    .map((i, n) => {
      const what = i.kind === "replace" ? `replaced ${q(i.old)} with ${q(i.new)}` : i.kind === "insert" ? `inserted ${q(i.new ?? i.text)}` : i.kind === "delete" ? `deleted ${q(i.old ?? i.text)}` : `${i.label ?? "formatting"} on ${q(i.text)}`;
      return `#${n + 1} [${i.author}, §${section(i)}] ${what}\n   context: …${i.before ?? ""}[HERE]${i.after ?? ""}…`;
    })
    .join("\n");
  const system = `You help ${me} review other people's tracked changes in their document. For every numbered change, suggest accept, reject or check:
- accept: it fixes an error, improves clarity or is a harmless style choice consistent with the rest of the document.
- reject: it introduces an error, contradicts the rest of the document (figures, names, terms, dates, cross-references), changes the meaning without reason, or conflicts with another change.
- check: it depends on facts or intentions you can't verify from the document (new figures, claims, commitments), or reviewers disagree.
Look at the document as a whole: a number changed in one place but not in another, a term renamed inconsistently, two people editing the same passage.
Answer only with one line per change, in order, the reason at most 15 words in the language of the document. The reader doesn't see the numbers: to point at another change, quote its words.
<advice id="1" do="accept">reason</advice>`;
  const prompt = `<document>\n${doc.slice(0, 120000)}\n</document>\n\n<changes>\n${list}\n</changes>`;
  return { system, prompt, maxTokens: Math.min(32000, 400 + changes.length * 60) };
}

/** Advice per item key for the changes in `items` (comments are left to the
 *  person). `onProgress` gets the number of changes judged so far. */
export async function reviewAdvice(host: AIHost, items: Item[], onProgress?: (n: number) => void): Promise<Map<string, Advice>> {
  const index = host.index();
  const changes = items.filter((i) => i.kind !== "comment").slice(0, ADVICE_MAX);
  const sec = (i: Item) => (i.section >= 0 ? index.sections[i.section].title : "-");
  const r = advicePrompt(host.author(), docMarkdown(host.view.state.doc), changes, sec);
  const out = await runAI(r.system, r.prompt, onProgress && ((t) => onProgress((t.match(/<\/advice>/g) ?? []).length)), r.maxTokens);
  return parseAdvice(out, changes);
}

export function parseAdvice(out: string, changes: Item[]): Map<string, Advice> {
  const m = new Map<string, Advice>();
  for (const [, id, d, why] of out.matchAll(/<advice\s+id="(\d+)"\s+do="(accept|reject|check)"\s*>([\s\S]*?)<\/advice>/g)) {
    const it = changes[Number(id) - 1];
    if (it) m.set(it.key, { do: d as Advice["do"], why: why.trim() });
  }
  return m;
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
