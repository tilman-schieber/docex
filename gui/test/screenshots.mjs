// Regenerates the README screenshots (docs/screenshots/*.png, 1440x900) in
// headless Chromium on the real backend.
//   npm run screenshots
import { join } from "node:path";
import { start, scratch, root } from "./browser.mjs";

const out = (name) => join(root, "docs/screenshots", name);
const { page, dir, close } = await start({ width: 1440, height: 900 });

/** Mouse off anything with a hover card; the file name instead of its scratch path. */
async function shot(name) {
  await page.mouse("mouseMoved", 1250, 888);
  await page.eval(`(() => { const p = document.querySelector("#status .path"); if (p) p.textContent = p.textContent.split("/").pop(); })()`);
  await new Promise((r) => setTimeout(r, 200));
  await page.screenshot(out(name));
}

try {
  // The review cockpit on a report edited by three people.
  await page.open(scratch(dir, "review.docx"));
  await shot("main.png");

  // Triage, a few items in.
  await page.click(".ProseMirror p", "Partner A");
  await page.key("t", "Alt");
  await page.waitFor(`document.querySelector(".triage")`);
  await page.key("s");
  await page.key("a");
  await page.settle();
  await shot("triage.png");
  await page.key("Escape");

  // The command palette.
  await page.key("p", "Control");
  await page.type("bob");
  await shot("palette.png");
  await page.key("Escape");

  // One reviewer's changes.
  await page.click("#sidebar .chip", "Alice");
  await page.settle();
  await shot("person.png");

  // A table with merged cells and the table toolbar.
  await page.open(scratch(dir, "merged-cells.docx"));
  await page.click(".ProseMirror td", "WP2 Evaluation");
  await shot("tables.png");

  // A chart and a SmartArt diagram, drawn from the document's data.
  await page.open(scratch(dir, "charts.docx"));
  await shot("charts.png");
} finally {
  await close();
}
console.log("docs/screenshots/ updated");
