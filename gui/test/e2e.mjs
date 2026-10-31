// End-to-end: real clicks and keys in the editor (headless Chromium) on the
// real backend, checking the document docex saves.
//   npm run e2e
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
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
    // Double-click the word itself (the paragraph's middle may be past its end).
    const [wx, wy] = await page.eval(`(() => {
      const p = [...document.querySelectorAll(".ProseMirror p")].find((e) => e.textContent.includes("split by work package"));
      const t = [...p.childNodes].find((n) => n.nodeType === 3 && n.textContent.includes("work"));
      const r = document.createRange();
      const i = t.textContent.indexOf("work");
      r.setStart(t, i);
      r.setEnd(t, i + 4);
      const b = r.getBoundingClientRect();
      return [b.left + b.width / 2, b.top + b.height / 2];
    })()`);
    await page.mouse("mouseMoved", wx, wy);
    await page.mouse("mousePressed", wx, wy, { clickCount: 1 });
    await page.mouse("mouseReleased", wx, wy, { clickCount: 1 });
    await page.mouse("mousePressed", wx, wy, { clickCount: 2 });
    await page.mouse("mouseReleased", wx, wy, { clickCount: 2 });
    await page.settle();
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

  await scenario("compare with a version sent back without tracked changes", async () => {
    const f = scratch(dir, "review.docx");
    // Their version: the same document, edited directly.
    const theirs = join(dir, "theirs.docx");
    execFileSync("cp", [join(root, "tests/fixtures/review.docx"), theirs]);
    execFileSync(join(root, "target/debug/docex"), ["accept", theirs, "--all", "--no-backup"], { stdio: "ignore" });
    const md = execFileSync(join(root, "target/debug/docex"), ["cat", theirs], { encoding: "utf8" }).replace("The team completed", "The team finished");
    writeFileSync(join(dir, "theirs.md"), md);
    execFileSync(join(root, "target/debug/docex"), ["apply", theirs, join(dir, "theirs.md"), "--untracked"], { stdio: "ignore" });
    await page.open(f);
    const count = () => page.eval(`document.querySelectorAll(".ProseMirror ins, .ProseMirror del").length`);
    const before = await count();
    await page.eval(`window.__DOCEX_DEV__.openPath = ${JSON.stringify(theirs)}`);
    await page.key("p", "Control");
    await page.type("compare with");
    await page.key("Enter");
    await page.waitFor(`document.querySelector("#modal:not([hidden]) input")`);
    await page.type("Bea Other");
    await page.key("Enter");
    await page.waitFor(`[...document.querySelectorAll(".ProseMirror ins")].some((e) => e.textContent === "finished" && e.dataset.author === "Bea Other")`);
    check(await page.eval(`[...document.querySelectorAll(".ProseMirror del")].some((e) => e.textContent === "completed" && e.dataset.author === "Bea Other")`), "the old word should show as deleted by Bea Other");
    check((await count()) === before + 2, `only the one replacement is new (${before} → ${await count()})`);
    check((await toast()).includes("by Bea Other, tracked"), `toast: ${await toast()}`);
    await page.click("#toasts .toast-act", "Undo");
    await page.settle();
    check((await count()) === before, "Undo takes the comparison back");
  });

  await scenario("triage with AI suggestions: preset, shown on the card, applied together", async () => {
    const f = scratch(dir, "review.docx");
    await page.open(f);
    const count = () => page.eval(`document.querySelectorAll(".ProseMirror ins, .ProseMirror del").length`);
    const before = await count();
    writeFileSync(join(dir, "ai.txt"), '<advice id="1" do="accept">Fixes the spelling.</advice>\n<advice id="2" do="reject">Contradicts section 3.</advice>\n<advice id="3" do="check">New figure; verify the source.</advice>');
    await page.key("p", "Control");
    await page.type("triage with ai");
    await page.key("Enter");
    await page.waitFor(`document.querySelector(".triage .tr-ai")`);
    const card = await page.eval(`document.querySelector(".triage .tr-ai").textContent`);
    check(card.includes("AI: accept") && card.includes("Fixes the spelling."), `card: ${card}`);
    check(await page.eval(`document.querySelector(".triage .tr-tally").textContent.startsWith("1 ✓  1 ✗")`), "accept and reject are preset");
    await page.key("Enter"); // to the summary
    await page.key("Enter"); // apply
    await page.settle();
    check(!(await page.eval(`!!document.querySelector(".triage")`)), "triage closes");
    check((await count()) < before, `the two decided changes are gone (${before} → ${await count()})`);
  });

  await scenario("distraction-free mode: changes marked, as if accepted, back", async () => {
    const f = scratch(dir, "review.docx");
    await page.open(f);
    const shown = (sel) => page.eval(`getComputedStyle(document.querySelector(${JSON.stringify(sel)})).display !== "none"`);
    const delShown = () => page.eval(`[...document.querySelectorAll(".ProseMirror del")].some((e) => getComputedStyle(e).display !== "none")`);
    const insPlain = () => page.eval(`(() => { const e = document.querySelector(".ProseMirror ins.chg"); const s = getComputedStyle(e); return s.color === getComputedStyle(e.closest("p, li, td, .ProseMirror")).color && !s.textDecorationLine.includes("underline"); })()`);
    await page.click(".ProseMirror p");
    await page.key("F11");
    check(await page.eval(`document.body.classList.contains("zen")`), "F11 enters");
    check(!(await shown("#toolbar")) && !(await shown("#sidebar")) && !(await shown("#status")), "only the page shows");
    check(await delShown(), "changes stay marked");
    check(await page.eval(`window.__DOCEX_DEV__.calls.includes("plugin:window|set_fullscreen")`), "the window goes full screen");
    await page.key("v", "Alt");
    check(!(await delShown()), "as if accepted: deletions hidden");
    check(await insPlain(), "as if accepted: insertions read as plain text");
    await page.click(".ProseMirror ins.chg");
    check(await page.eval(`document.getElementById("popover").hidden`), "no popover on accepted-looking text");
    await page.key("Escape");
    check(!(await page.eval(`document.body.classList.contains("zen")`)), "Esc leaves");
    check((await shown("#toolbar")) && (await delShown()), "toolbar and markup are back");
    await page.key("F11");
    check(await page.eval(`document.body.classList.contains("zen-clean")`), "F11 comes back to the last view");
    await page.key("F11");
  });

  // Pictures. rich.docx has one: "A red box", 1.4 × 0.7 cm.
  const images = (f) => JSON.parse(execFileSync(join(root, "target/debug/docex"), ["images", f, "--json"], { encoding: "utf8" }));
  const trackOn = async () => {
    await page.key("p", "Control");
    await page.type("track changes: turn on");
    await page.key("Enter");
    await page.waitFor(`document.querySelector(".track-switch").classList.contains("on")`);
    await page.settle();
  };
  const saveTo = async (f) => {
    await page.key("s", "Control");
    await page.waitFor(`!document.querySelector("#status .dirty")`);
    return images(f);
  };

  await scenario("picture menu: alt text, resize by the corner, saved untracked", async () => {
    const f = scratch(dir, "rich.docx");
    await page.open(f);
    await page.click(".ProseMirror .imgv img");
    await page.waitFor(`!document.getElementById("popover").hidden`);
    const pop = await page.eval(`document.getElementById("popover").textContent`);
    check(pop.includes("1.4 × 0.7 cm") && pop.includes("A red box"), `menu: ${pop}`);
    await page.click("#popover button", "Alt text");
    await page.waitFor(`document.querySelector("#modal:not([hidden]) textarea")`);
    await page.eval(`document.querySelector("#modal textarea").select()`);
    await page.type("A small red rectangle");
    await page.key("Enter", "Control");
    // Drag the corner 53px → 106px wide.
    await page.click(".ProseMirror .imgv img");
    await page.key("Escape");
    const [x, y] = await page.center(".imgv-grip");
    await page.mouse("mouseMoved", x, y);
    await page.mouse("mousePressed", x, y, { buttons: 1 });
    for (let k = 1; k <= 5; k++) await page.mouse("mouseMoved", x + k * 10.6, y, { buttons: 1 });
    await page.mouse("mouseReleased", x + 53, y, { buttons: 0 });
    await page.settle();
    const [im] = await saveTo(f);
    check(im.alt === "A small red rectangle", `alt: ${im.alt}`);
    check(Math.abs(im.width_emu - 1016000) < 20000 && Math.abs(im.height_emu - 508000) < 20000, `size: ${im.width_emu} × ${im.height_emu}`);
    check(im.status === "plain", "alt text and size aren't tracked changes");
  });

  await scenario("replace a picture: tracked, same width, its own proportions", async () => {
    const f = scratch(dir, "rich.docx");
    await page.open(f);
    await trackOn();
    await page.eval(`window.__DOCEX_DEV__.openPath = ${JSON.stringify(join(root, "tests/fixtures/tall.png"))}`);
    await page.click(".ProseMirror .imgv img");
    await page.click("#popover button", "Replace");
    await page.waitFor(`document.querySelectorAll(".ProseMirror .imgv").length === 2`);
    check(await page.eval(`!!document.querySelector(".ProseMirror del .imgv") && !!document.querySelector(".ProseMirror ins .imgv")`), "old struck through, new inserted");
    const ims = await saveTo(f);
    const old = ims.find((i) => i.status === "del");
    const neu = ims.find((i) => i.status === "ins");
    check(old && neu, `pictures: ${JSON.stringify(ims)}`);
    check(neu.width_emu === 508000 && neu.height_emu === 1016000 && neu.alt === "A red box", `new: ${JSON.stringify(neu)}`);
  });

  await scenario("paste a screenshot, drop a picture, AI writes the alt text", async () => {
    const f = scratch(dir, "rich.docx");
    await page.open(f);
    await trackOn();
    await page.click(".ProseMirror p", "Second goal");
    await page.key("End");
    // A 40×20 PNG on the clipboard, nothing else.
    await page.eval(`(() => {
      const b64 = "iVBORw0KGgoAAAANSUhEUgAAACgAAAAUCAIAAABwJOjsAAAAJElEQVR4nGMQqbgzIIhh1OJRi0ctHrV41OJRi0ctHrV45FgMADHwZT1J/KBdAAAAAElFTkSuQmCC";
      const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      const dt = new DataTransfer();
      dt.items.add(new File([bytes], "shot.png", { type: "image/png" }));
      document.querySelector(".ProseMirror").dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
    })()`);
    await page.waitFor(`document.querySelectorAll(".ProseMirror .imgv").length === 2`);
    // Dropped on the first paragraph.
    const [x, y] = await page.center(".ProseMirror p", "First goal");
    const dpr = await page.eval("window.devicePixelRatio");
    await page.eval(`window.__DOCEX_DEV__.emit("tauri://drag-drop", { paths: [${JSON.stringify(join(root, "tests/fixtures/tall.png"))}], position: { x: ${x * dpr}, y: ${y * dpr} } })`);
    await page.waitFor(`document.querySelectorAll(".ProseMirror .imgv").length === 3`);
    check(await page.eval(`[...document.querySelectorAll(".ProseMirror p")].find((p) => p.textContent.includes("First goal")).querySelector(".imgv") !== null`), "the dropped picture lands where it was dropped");
    writeFileSync(join(dir, "ai.txt"), "A blue bar");
    await page.key("p", "Control");
    await page.type("alt text for pictures");
    await page.key("Enter");
    await page.waitFor(`document.querySelector("#modal:not([hidden]) .alt-list")`);
    check((await page.eval(`document.querySelectorAll("#modal .alt-row").length`)) === 2, "two pictures without alt text");
    const req = JSON.parse(readFileSync(join(dir, "ai.txt.req"), "utf8"));
    check(req.images.length === 1 && req.images[0].startsWith("data:image/png;base64,"), "the picture goes to the AI");
    check(req.prompt.includes("Second goal") || req.prompt.includes("First goal"), `context: ${req.prompt}`);
    await page.click("#modal button", "Use these");
    const ims = await saveTo(f);
    check(ims.length === 3 && ims.every((i) => i.alt), `alt texts: ${JSON.stringify(ims.map((i) => i.alt))}`);
    check(ims.filter((i) => i.status === "ins").length === 2, "pasted and dropped pictures are tracked insertions");
  });

  await scenario("a picture the editor can't show gets a named box", async () => {
    await page.eval(`(() => { const i = document.querySelector(".ProseMirror .imgv img"); i.src = "data:image/x-emf;base64,AAAA"; })()`);
    await page.waitFor(`document.querySelector(".ProseMirror .imgv img").src.startsWith("data:image/svg+xml")`);
    check(decodeURIComponent(await page.eval(`document.querySelector(".ProseMirror .imgv img").src`)).includes("No preview"), "placeholder names the picture");
  });

  await scenario("dropping a Word document opens it", async () => {
    const other = scratch(dir, "review.docx");
    await page.eval(`window.__DOCEX_DEV__.emit("tauri://drag-drop", { paths: [${JSON.stringify(other)}], position: { x: 300, y: 300 } })`);
    await page.waitFor(`document.querySelector("#status .path").textContent.endsWith("review.docx")`);
  });

  await scenario("document look: preview a theme, cancel, apply, undo, saved as Word styles", async () => {
    const f = scratch(dir, "review.docx");
    await page.open(f);
    const look = () => page.eval(`document.getElementById("doc-look").textContent`);
    const own = await look();
    check(own.includes(".page.look") && (await page.eval(`document.getElementById("editor").classList.contains("look")`)), "the page shows the document's own look");
    const openPicker = async () => {
      await page.key("p", "Control");
      await page.type("document look");
      await page.key("Enter");
      await page.waitFor(`document.getElementById("looks")`);
    };
    await openPicker();
    await page.click("#looks .lk-row", "Plain");
    check((await look()).includes("Arial"), "previewing shows the theme on the page");
    await page.key("Escape");
    check(!(await page.eval(`document.getElementById("looks")`)) && (await look()) === own, "Esc goes back to the document's look");
    await openPicker();
    await page.click("#looks .lk-row", "Plain");
    await page.click("#looks button", "Apply");
    await page.waitFor(`document.getElementById("doc-look").textContent.includes("Arial") && !document.getElementById("looks")`);
    check((await toast()).includes("New look"), `toast: ${await toast()}`);
    await page.click("#toasts .toast-act", "Undo");
    await page.waitFor(`!document.getElementById("doc-look").textContent.includes("Arial")`);
    await openPicker();
    await page.click("#looks .lk-row", "Plain");
    await page.key("Enter");
    await page.waitFor(`document.getElementById("doc-look").textContent.includes("Arial")`);
    await page.key("s", "Control");
    await page.waitFor(`!document.querySelector("#status .dirty")`);
    const css = execFileSync(join(root, "target/debug/docex"), ["style", f], { encoding: "utf8" });
    check(css.includes("font-family: Arial") && css.includes("size: A4"), `saved look:\n${css}`);
    const md = execFileSync(join(root, "target/debug/docex"), ["cat", f], { encoding: "utf8" });
    check(md === execFileSync(join(root, "target/debug/docex"), ["cat", join(root, "tests/fixtures/review.docx")], { encoding: "utf8" }), "the text and its changes are untouched");
  });
} finally {
  await close();
}
console.log(failed ? `${failed} failed (screenshots in ${dir})` : "all passed");
process.exit(failed ? 1 : 0);
