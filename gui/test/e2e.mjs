// End-to-end: real clicks and keys in the editor (headless Chromium) on the
// real backend, checking the document docex saves.
//   npm run e2e
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { start, scratch, cat, root } from "./browser.mjs";

const { page, dir, close } = await start();
let failed = 0;

async function scenario(name, fn) {
  try {
    await fn();
    console.log(`ok   ${name}`);
  } catch (e) {
    failed++;
    console.log(`FAIL ${name}\n     ${String(e.message ?? e).split("\n").join("\n     ")}`);
    await page.screenshot(`${dir}/fail-${name.replace(/\W+/g, "-")}.png`).catch(() => {});
  }
}

function check(cond, msg) {
  if (!cond) throw new Error(msg);
}

const editorHas = (sel, text) => page.eval(`[...document.querySelectorAll(".ProseMirror ${sel}")].some((e) => e.textContent === ${JSON.stringify(text)})`);
const toast = () => page.eval(`[...document.querySelectorAll("#toasts .toast")].map((t) => t.textContent).join(" | ")`);

try {
  let file = scratch(dir, "review.docx");
  await page.open(file);

  await scenario("popover decides a whole replacement, toast undoes it", async () => {
    await page.click(".ProseMirror ins.chg", "recruiting");
    await page.waitFor(`!document.getElementById("popover").hidden`);
    const pop = await page.eval(`document.getElementById("popover").textContent`);
    check(pop.includes("Chen Wei") && pop.includes("work") && pop.includes("recruiting"), `popover: ${pop}`);
    await page.click("#popover button", "Accept");
    await page.settle();
    check(!(await editorHas("del.chg", "work")) && !(await editorHas("ins.chg", "recruiting")), "both halves should be accepted");
    check((await toast()).includes("Accepted work → recruiting"), `toast: ${await toast()}`);
    await page.click("#toasts .toast-act", "Undo");
    await page.settle();
    check((await editorHas("del.chg", "work")) && (await editorHas("ins.chg", "recruiting")), "undo should bring the change back");
  });

  await scenario("comment on a change from its popover", async () => {
    await page.click(".ProseMirror ins.chg", "summarizes");
    await page.click("#popover button", "Comment");
    await page.waitFor(`!document.getElementById("modal").hidden`);
    await page.type("Why US spelling?");
    await page.key("Enter", "Control");
    await page.settle();
    await page.waitFor(`[...document.querySelectorAll("#sidebar .item.comment")].some((e) => e.textContent.includes("Why US spelling?"))`);
  });

  await scenario("clicking commented text keeps the caret in the document", async () => {
    await page.click(".ProseMirror .cmt", "4,200 documents");
    await page.waitFor(`document.querySelector("#sidebar .item.comment.active")`);
    check(await page.eval(`document.activeElement.classList.contains("ProseMirror")`), "focus moved out of the editor");
  });

  await scenario("deleting a thread needs a second click", async () => {
    await page.click("#sidebar .item.comment", "deduplication");
    await page.click("#sidebar .item.comment.active .act.no");
    check(await page.eval(`!!document.querySelector("#sidebar .act.confirm")`), "no confirmation shown");
    check(await page.eval(`[...document.querySelectorAll("#sidebar .item.comment")].some((e) => e.textContent.includes("deduplication"))`), "deleted on the first click");
    await page.click("#sidebar .act.confirm");
    await page.settle();
    check(!(await page.eval(`[...document.querySelectorAll("#sidebar .item.comment")].some((e) => e.textContent.includes("deduplication"))`)), "thread still there");
  });

  await scenario("triage: accept one, apply, undo from the toast", async () => {
    const before = await page.eval(`document.querySelector(".summary b").textContent`);
    await page.click(".ProseMirror p", "Partner A");
    await page.key("t", "Alt");
    await page.waitFor(`document.querySelector(".triage")`);
    check(await page.eval(`document.querySelector(".triage").getBoundingClientRect().height < innerHeight * 0.6`), "triage should leave the page visible");
    await page.key("a");
    await page.key("Enter");
    await page.key("Enter");
    await page.settle();
    const after = await page.eval(`document.querySelector(".summary b").textContent`);
    check(Number(after) === Number(before) - 1, `changes ${before} -> ${after}`);
    check((await toast()).includes("Triage: 1 accepted"), `toast: ${await toast()}`);
    await page.key("z", "Control");
    await page.settle();
    check((await page.eval(`document.querySelector(".summary b").textContent`)) === before, "Ctrl+Z should undo the triage");
  });

  await scenario("save writes the decisions to the file", async () => {
    await page.click(".ProseMirror ins.chg", "roughly");
    await page.click("#popover button", "Reject");
    await page.settle();
    await page.key("s", "Control");
    await page.settle();
    const md = cat(file);
    check(md.includes("by about 12") && !md.includes("roughly"), md);
    check(md.includes("Why US spelling?") && !md.includes("deduplication"), "comment edits not saved");
  });

  file = scratch(dir, "merged-cells.docx");
  await page.open(file);

  await scenario("merge two cells by dragging and the toolbar", async () => {
    await page.drag(".ProseMirror td", "Staff", ".ProseMirror td", "Material");
    await page.waitFor(`document.querySelectorAll(".ProseMirror .selectedCell").length === 2`);
    await page.click('#toolbar [data-tip^="Merge"]');
    await page.settle();
    check(await page.eval(`[...document.querySelectorAll(".ProseMirror td")].some((td) => td.getAttribute("colspan") === "2" && td.textContent.startsWith("Staff"))`), "no merged cell");
    await page.key("s", "Control");
    await page.settle();
    const md = cat(file);
    check(md.includes('<td colspan="2">Staff<br>Material</td>'), md);
  });

  await scenario("add a column with the toolbar", async () => {
    await page.click(".ProseMirror td", "Total");
    await page.click('#toolbar [data-tip^="Insert a column on the right"]');
    await page.key("s", "Control");
    await page.settle();
    check(cat(file).includes("<td>Total</td>\n<td></td>"), cat(file));
  });

  await scenario("underline and highlight from the toolbar", async () => {
    await page.click(".ProseMirror p", "split by work package", { clickCount: 2 });
    await page.click('#toolbar [data-tip^="Underline"]');
    await page.click('#toolbar [data-tip^="Highlight"]');
    await page.key("s", "Control");
    await page.settle();
    const md = cat(file);
    check(/<u><mark>\w+<\/mark><\/u>|<mark><u>\w+<\/u><\/mark>|<u>\w+<\/u>/.test(md) && md.includes("<mark>"), md);
  });

  // A fresh copy, with a comment by Bob that mentions Tilman.
  file = scratch(dir, "review.docx");
  execFileSync(join(root, "target/debug/docex"), ["comment", file, "--on", "held-out", "@Tilman can you confirm the split?", "--author", "Bob Okafor"]);
  await page.open(file);

  await scenario("review progress and the done-reviewing summary", async () => {
    check((await page.eval(`document.querySelector(".progress-row").textContent`)).includes("0 of 19 reviewed"), "progress at start");
    await page.click(".ProseMirror ins.chg", "recruiting");
    await page.waitFor(`!document.getElementById("popover").hidden`);
    await page.click("#popover button", "Accept");
    await page.settle();
    await page.click(".ProseMirror ins.chg", "24");
    await page.waitFor(`!document.getElementById("popover").hidden && document.getElementById("popover").textContent.includes("24")`);
    await page.click("#popover button", "Reject");
    await page.settle();
    check((await page.eval(`document.querySelector(".progress-row").textContent`)).includes("2 of 19 reviewed"), await page.eval(`document.querySelector(".progress-row").textContent`));
    await page.click(".progress-row button", "summary");
    await page.waitFor(`document.querySelector(".summary-text")`);
    const text = await page.eval(`document.querySelector(".summary-text").value`);
    for (const want of ["I went through 2 of 19 changes: 1 accepted, 1 rejected, 17 still open.", 'not taken: "twenty" → "24" (Next steps)', "Chen Wei: 1 accepted", "Still open (4):"]) {
      check(text.includes(want), `missing ${JSON.stringify(want)} in\n${text}`);
    }
    await page.key("Escape");
    await page.key("z", "Control");
    await page.settle();
    check((await page.eval(`document.querySelector(".progress-row").textContent`)).includes("1 of 19 reviewed"), "undo should take the decision out of the progress");
  });

  await scenario("threads waiting on me", async () => {
    await page.click("#sidebar .pill", "For me");
    const items = await page.eval(`[...document.querySelectorAll("#feed .item.comment")].map((e) => e.textContent)`);
    check(items.length === 1 && items[0].includes("can you confirm") && items[0].includes("waiting on you"), JSON.stringify(items));
    await page.click("#feed .item.comment", "can you confirm");
    await page.click("#feed .reply-box textarea");
    await page.type("Yes, 500 held out.");
    await page.key("Enter", "Control");
    await page.settle();
    check(!(await page.eval(`[...document.querySelectorAll("#sidebar .pill")].some((p) => p.textContent.includes("For me"))`)), "answered thread still waiting on me");
  });

  await scenario("find and replace, tracked", async () => {
    const f = scratch(dir, "review.docx");
    await page.open(f);
    const count = () => page.eval(`document.querySelector(".fb-count").textContent`);
    await page.click(".ProseMirror p", "Partner A");
    await page.key("f", "Control");
    await page.waitFor(`!document.querySelector(".findbar").hidden`);
    await page.type("KI");
    check((await count()) === "no matches", `deleted text matched: ${await count()}`);
    await page.eval(`(() => { const i = document.querySelector(".fb-find"); i.value = ""; i.dispatchEvent(new Event("input")); })()`);
    await page.type("partner");
    check((await count()).endsWith("of 4"), `partner: ${await count()}`);
    const first = await count();
    await page.key("Enter");
    check((await count()) !== first, "Enter should go to the next match");
    // Replace all: four tracked replacements, one undo step.
    await page.key("h", "Control");
    await page.waitFor(`document.querySelector(".findbar").classList.contains("replacing")`);
    await page.type("client");
    await page.key("Enter", "Control");
    const n = () => page.eval(`[...document.querySelectorAll(".ProseMirror ins.chg")].filter((e) => e.textContent === "client").length`);
    check((await n()) === 4, `tracked insertions: ${await n()}`);
    check((await count()) === "no matches", `after replace all: ${await count()}`);
    await page.key("Escape");
    await page.key("z", "Control");
    check((await n()) === 0, "Ctrl+Z should take back all four");
    await page.key("y", "Control");
    check((await n()) === 4, "Ctrl+Y should redo them");
    // One at a time.
    await page.key("h", "Control");
    await page.eval(`(() => { const i = document.querySelector(".fb-find"); i.value = "held-out"; i.dispatchEvent(new Event("input")); })()`);
    await page.click(".fb-replace");
    await page.eval(`(() => { const i = document.querySelector(".fb-replace"); i.value = ""; })()`);
    await page.type("hold-out");
    await page.key("Enter");
    await page.key("Escape");
    await page.key("s", "Control");
    await page.settle();
    const md = cat(f);
    check(md.includes("{--Partner--}{++client++} A supplied") && md.includes("three {--partner--}{++client++}"), md);
    check(md.includes("{--held-out--}{++hold-out++}"), md);
  });

  await scenario("AI edits the Markdown: a real link and bold, only in the selection, undoable", async () => {
    const f = scratch(dir, "rich.docx");
    await page.open(f);
    writeFileSync(
      join(dir, "ai.txt"),
      `<edit>
<find>- Bullet one</find>
<replace>- Bullet one, see the [docs](https://docs.example.com)</replace>
</edit>
<edit>
<find>Bullet two</find>
<replace>**Bullet** two</replace>
</edit>
<edit>
<find>## Goals</find>
<replace>## Aims</replace>
</edit>`,
    );
    await page.click(".ProseMirror p", "Bullet one");
    await page.key("j", "Control");
    await page.type("tighten");
    await page.key("Enter");
    await page.waitFor(`[...document.querySelectorAll("#toasts .toast")].some((t) => t.textContent.includes("suggestion"))`);
    await page.settle();
    check(await page.eval(`[...document.querySelectorAll(".ProseMirror a")].some((a) => a.textContent === "docs" && (a.closest("ins") || a.querySelector("ins")))`), "the link should be a real, tracked link");
    check(await editorHas("strong", "Bullet"), "Bullet should be bold");
    check(await page.eval(`document.querySelector(".ProseMirror").textContent.includes("Goals")`), "edits outside the selection must be left out");
    check((await toast()).includes("1 of 3 edits didn't match"), `toast: ${await toast()}`);
    await page.key("s", "Control");
    await page.settle();
    const md = cat(f);
    check(md.includes("[{++docs++}](https://docs.example.com)") || md.includes("{++see the [docs](https://docs.example.com)++}") || /\{\+\+[^+]*\[docs\]\(https:\/\/docs\.example\.com\)/.test(md), md);
    check(md.includes("## Goals"), md);
    // The bolding is a formatting change placed in the text, not "not in the text".
    check(await editorHas(".fmtchg", "Bullet"), "the formatting change should be marked in the text");
    check(!(await page.eval(`document.querySelector("#sidebar").textContent.includes("Not in the text")`)), "formatting change listed as not in the text");
    await page.click(".ProseMirror .fmtchg", "Bullet");
    await page.waitFor(`!document.getElementById("popover").hidden`);
    check((await page.eval(`document.getElementById("popover").textContent`)).includes("formatting"), "popover for the formatting change");
    await page.key("Escape");
    // Undo takes the whole AI edit back.
    await page.key("z", "Control");
    await page.settle();
    check(!(await page.eval(`!!document.querySelector(".ProseMirror a[href='https://docs.example.com']")`)), "undo should remove the AI's edit");
  });

  await scenario("unsaved edits survive a crash", async () => {
    const f = scratch(dir, "charts.docx");
    await page.open(f);
    await page.click(".ProseMirror p", "Revenue grew");
    await page.key("End");
    await page.type(" Recovered text.");
    await new Promise((r) => setTimeout(r, 5000));
    // "Crash": the page goes away without saving; open the file again.
    await page.open(f);
    check(!cat(f).includes("Recovered text."), "the file itself must be unchanged");
    await page.waitFor(`document.querySelector(".ProseMirror").textContent.includes("Recovered text.")`);
    check((await toast()).includes("restored"), `toast: ${await toast()}`);
    check(await page.eval(`!!document.querySelector("#status .dirty")`), "restored edits should count as unsaved");
    await page.key("s", "Control");
    await page.settle();
    check(cat(f).includes("Recovered text."), "save should write the restored edits");
    await page.open(f);
    check(!(await toast()).includes("restored"), "nothing to restore after saving");
  });

  await scenario("a newer release shows in the status bar until dismissed", async () => {
    const f = scratch(dir, "review.docx");
    const notice = () => page.eval(`document.querySelector("#status .update")?.textContent ?? ""`);
    await page.open(f);
    check(!(await notice()), "no notice without a newer release");
    writeFileSync(join(dir, "update.json"), JSON.stringify({ version: "9.9.9", url: "https://github.com/tilman-schieber/docex/releases/tag/v9.9.9" }));
    // The check runs once a day: forget today's.
    await page.eval(`localStorage.removeItem("docex-update")`);
    await page.open(f);
    await page.waitFor(`document.querySelector("#status .update")`);
    check((await notice()).includes("docex 9.9.9 is available"), `notice: ${await notice()}`);
    await page.click("#status .update .get");
    check(await page.eval(`window.__DOCEX_DEV__.calls.includes("open_release")`), "clicking should open the release page");
    await page.click("#status .update .x");
    check(!(await notice()), "× hides the notice");
    await page.eval(`localStorage.removeItem("docex-update")`);
    await page.open(f);
    await page.settle();
    check(!(await notice()), "a dismissed version stays hidden");
  });
} finally {
  await close();
}
console.log(failed ? `${failed} failed (screenshots in ${dir})` : "all passed");
process.exit(failed ? 1 : 0);
