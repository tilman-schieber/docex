// Runs the GUI's conversion and tracking code against documents, using the
// docex core (via the `modelcheck` example) as the oracle.
//   npm test -- FILE.docx...   (defaults to the repo's fixtures)
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const root = join(import.meta.dirname, "..", "..");
const files = process.argv.slice(2).map((f) => resolve(f));
if (!files.length) files.push(...["rich.docx", "math.docx", "pandoc-math.docx", "merged-cells.docx", "content-controls.docx", "formatting.docx", "header-footer.docx", "charts.docx"].map((f) => join(root, "tests/fixtures", f)));
const dir = mkdtempSync(join(tmpdir(), "docex-gui-test-"));
const cargo = (...a) => execFileSync("cargo", ["run", "-q", "-p", "docex-core", "--example", "modelcheck", "--", ...a], { cwd: root, encoding: "utf8" });

let failed = 0;
for (const f of files) {
  const m = join(dir, "m.json");
  const m2 = join(dir, "m2.json");
  cargo("dump", f, m);
  execFileSync("node", [join(import.meta.dirname, "..", "test-dist", "roundtrip", "roundtrip.js"), m, m2]);
  const out = cargo("check", f, m2).trim();
  const ok = out.includes("changed=false identical=true");
  if (!ok) failed++;
  console.log(`${ok ? "ok  " : "FAIL"} round trip ${out}`);
}

// Live tracking: typing, own-insertion backspace, replacing a selection.
const t = join(dir, "t.docx");
writeFileSync(join(dir, "t.md"), "# T\n\nFirst line.\n\nSecond clause stays here.\n");
execFileSync("cargo", ["run", "-q", "-p", "docex", "--", "new", join(dir, "t.md"), "-o", t], { cwd: root, stdio: "ignore" });
cargo("dump", t, join(dir, "t.json"));
execFileSync("node", [join(import.meta.dirname, "..", "test-dist", "track", "track.js"), join(dir, "t.json"), join(dir, "t2.json")], { stdio: "ignore" });
const md = execFileSync("cargo", ["run", "-q", "-p", "docex-core", "--example", "applymodel", "--", t, join(dir, "t2.json")], { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
const want = "Second{++ bra++} clause {--stays--}{++remains++} here.";
const ok = md.includes(want);
if (!ok) failed++;
console.log(`${ok ? "ok  " : "FAIL"} live tracking -> ${md.trim().split("\n").pop()}`);
// Table structure from the editor: merge two cells, add a column.
const mc = join(root, "tests/fixtures/merged-cells.docx");
cargo("dump", mc, join(dir, "mc.json"));
execFileSync("node", [join(import.meta.dirname, "..", "test-dist", "tables", "tables.js"), join(dir, "mc.json"), join(dir, "mc2.json")]);
const tmd = execFileSync("cargo", ["run", "-q", "-p", "docex-core", "--example", "applymodel", "--", mc, join(dir, "mc2.json")], { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
const tok = tmd.includes('<td colspan="2">Staff<br>{++Material++}</td>') && tmd.includes("<td>WP1 Data pipeline</td>\n<td></td>\n<td>40,000</td>");
if (!tok) failed++;
console.log(`${tok ? "ok  " : "FAIL"} table merge + new column${tok ? "" : "\n" + tmd}`);
process.exit(failed ? 1 : 0);
