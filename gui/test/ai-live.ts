// Tries AI edits against the real model (costs a few cents; needs
// ANTHROPIC_API_KEY): each case edits a scratch copy of a fixture through
// `docex apply --track` and prints the model's answer and the review.
//   npm run ai-live
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { applyEdits, EDIT_SYSTEM, editPrompt, parseEdits } from "../src/ai";

const root = join(import.meta.dirname, "..", "..", "..");
const out = mkdtempSync(join(tmpdir(), "docex-ai-"));
const docex = (...a: string[]) => execFileSync(join(root, "target/debug/docex"), a, { encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] });

async function ask(prompt: string): Promise<string> {
  const r = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: { "x-api-key": process.env.ANTHROPIC_API_KEY!, "anthropic-version": "2026-10-21", "content-type": "application/json" },
    body: JSON.stringify({ model: "claude-opus-5-5", max_tokens: 32000, system: EDIT_SYSTEM, output_config: { effort: "medium" }, messages: [{ role: "user", content: prompt }] }),
  });
  const j = await r.json();
  if (!r.ok) throw new Error(JSON.stringify(j));
  return j.content.filter((c: { type: string }) => c.type === "text").map((c: { text: string }) => c.text).join("");
}

const cases: { fixture: string; region?: string; instruction: string }[] = [
  { fixture: "rich.docx", region: "This is the *introduction*", instruction: "Make the word 'introduction' a link to https://example.org/intro and underline 'second line'." },
  { fixture: "rich.docx", instruction: "Turn the quoted paragraph into a level-3 heading, and make 'Bob' bold in the table." },
  { fixture: "review.docx", region: "Partner A", instruction: "Tighten." },
];

for (const [k, c] of cases.entries()) {
  const file = join(out, `case${k}-${c.fixture}`);
  copyFileSync(join(root, "tests/fixtures", c.fixture), file);
  const md = docex("cat", file);
  let region: [number, number] | null = null;
  if (c.region) {
    const at = md.indexOf(c.region);
    region = [md.lastIndexOf("\n\n", at) + 2, md.indexOf("\n\n", at) < 0 ? md.length : md.indexOf("\n\n", at)];
  }
  const answer = await ask(editPrompt(md, region, c.instruction));
  const { edits, note } = parseEdits(answer);
  const r = applyEdits(md, edits, region);
  writeFileSync(`${file}.md`, r.md);
  let report = "";
  try {
    docex("apply", file, `${file}.md`, "--track", "--author", "AI test");
    report = docex("review", file, "--by", "AI test");
  } catch (e) {
    report = String((e as { stderr?: string }).stderr ?? e);
  }
  console.log(`=== ${k}: ${c.instruction}\n--- answer:\n${answer}\n--- ${r.applied} applied, ${r.failed.length} failed${note ? `, note: ${note}` : ""}\n--- review:\n${report}`);
}
