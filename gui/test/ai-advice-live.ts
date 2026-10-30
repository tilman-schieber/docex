// Tries the AI's triage advice against the real model (costs a few cents;
// needs ANTHROPIC_API_KEY): the changes of a fixture, as the app sends them.
//   npm run ai-advice-live [-- fixture.docx]
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { advicePrompt, parseAdvice } from "../src/ai";
import type { Item } from "../src/analyze";

const root = join(import.meta.dirname, "..", "..", "..");
const file = join(root, "tests/fixtures", process.argv[2] ?? "review.docx");
const docex = (...a: string[]) => execFileSync(join(root, "target/debug/docex"), a, { encoding: "utf8" });

type Json = { id: string; kind: string; author: string; section: string[]; before?: string; old?: string; new?: string; after?: string; text?: string };
const items = (JSON.parse(docex("review", file, "--json")).items as Json[])
  .filter((i) => i.kind !== "comment")
  .map((i) => ({ ...i, key: i.id, section: 0, path: i.section.join(" › ") }) as unknown as Item & { path: string });
const r = advicePrompt("Tilman Schieber", docex("cat", file, "--accepted"), items, (i) => (i as Item & { path: string }).path);
const res = await fetch("https://api.anthropic.com/v1/messages", {
  method: "POST",
  headers: { "x-api-key": process.env.ANTHROPIC_API_KEY!, "anthropic-version": "2026-10-21", "content-type": "application/json" },
  body: JSON.stringify({ model: "claude-opus-5-5", max_tokens: r.maxTokens, system: r.system, output_config: { effort: "medium" }, messages: [{ role: "user", content: r.prompt }] }),
});
const j = await res.json();
if (!res.ok) throw new Error(JSON.stringify(j));
const out: string = j.content.filter((c: { type: string }) => c.type === "text").map((c: { text: string }) => c.text).join("");
const advice = parseAdvice(out, items);
for (const i of items) {
  const a = advice.get(i.key);
  const what = i.kind === "replace" ? `${i.old} → ${i.new}` : (i.new ?? i.old ?? i.text ?? i.kind);
  console.log(`${i.key.padEnd(4)} ${(a?.do ?? "—").padEnd(7)} ${i.author.padEnd(12)} ${what.slice(0, 40).padEnd(40)} ${a?.why ?? ""}`);
}
console.log(`\n${advice.size} of ${items.length} changes judged`);
