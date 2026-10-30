// Drives the editor in headless Chromium on the real backend, for end-to-end
// tests and README screenshots. No dependencies beyond Vite: Chromium is
// driven over the DevTools protocol (Node's built-in WebSocket), the backend
// is the `devserver` binary, the page is dev.html.
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, basename } from "node:path";
import { createServer } from "vite";

export const root = join(import.meta.dirname, "..", "..");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** A scratch copy of a fixture (tests and screenshots never touch the originals). */
export function scratch(dir, fixture) {
  const to = join(dir, basename(fixture));
  copyFileSync(join(root, "tests/fixtures", fixture), to);
  return to;
}

/** The docex CLI's Markdown for a file (to check what was saved). */
export function cat(file) {
  return execFileSync(join(root, "target/debug/docex"), ["cat", file], { encoding: "utf8" });
}

function waitForLine(proc, re, what) {
  return new Promise((resolve, reject) => {
    let buf = "";
    const on = (d) => {
      buf += d;
      const m = buf.match(re);
      if (m) resolve(m);
    };
    proc.stdout?.on("data", on);
    proc.stderr?.on("data", on);
    proc.on("exit", (code) => reject(new Error(`${what} exited (${code}):\n${buf}`)));
  });
}

/** Start dev server, Vite and Chromium. Returns `{ page, dir, close }`. */
export async function start({ width = 1440, height = 900 } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "docex-e2e-"));
  execFileSync("cargo", ["build", "-q", "-p", "docex-gui", "--bin", "devserver", "-p", "docex"], { cwd: root, stdio: "inherit" });
  const port = 1431 + Math.floor(Math.random() * 500);
  // Snapshots and AI settings go to the scratch dir, not the user's.
  const env = { ...process.env, XDG_DATA_HOME: join(dir, "data"), XDG_CONFIG_HOME: join(dir, "config"), DOCEX_AUTHOR: "Tilman Schieber" };
  const backend = spawn(join(root, "target/debug/devserver"), [String(port)], { env, stdio: ["ignore", "ignore", "pipe"] });
  await waitForLine(backend, /dev server on/, "devserver");
  process.env.DOCEX_DEVSERVER_PORT = String(port);
  const vite = await createServer({ root: join(root, "gui"), configFile: join(root, "gui/vite.config.ts"), logLevel: "error", server: { port: 0, strictPort: false } });
  await vite.listen();
  const base = vite.resolvedUrls.local[0];
  const chrome = spawn("chromium", ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${join(dir, "chrome")}`, "--no-first-run", "--hide-scrollbars", `--window-size=${width},${height}`, "about:blank"], { stdio: ["ignore", "ignore", "pipe"] });
  const [, ws] = await waitForLine(chrome, /DevTools listening on (ws:\S+)/, "chromium");
  const page = await Page.connect(ws, base, width, height);
  return {
    page,
    dir,
    async close() {
      page.socket.close();
      chrome.kill();
      backend.kill();
      await vite.close();
    },
  };
}

class Page {
  static async connect(wsUrl, base, width, height) {
    const p = new Page();
    p.base = base;
    p.socket = new WebSocket(wsUrl);
    p.pending = new Map();
    p.id = 0;
    p.events = [];
    await new Promise((r) => p.socket.addEventListener("open", r, { once: true }));
    p.socket.addEventListener("message", (m) => {
      const msg = JSON.parse(m.data);
      if (msg.id && p.pending.has(msg.id)) {
        const { resolve, reject } = p.pending.get(msg.id);
        p.pending.delete(msg.id);
        msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
      } else if (msg.method) p.events.push(msg);
    });
    const { targetId } = await p.send("Target.createTarget", { url: "about:blank" });
    const { sessionId } = await p.send("Target.attachToTarget", { targetId, flatten: true });
    p.session = sessionId;
    await p.send("Page.enable");
    await p.send("Runtime.enable");
    await p.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
    return p;
  }

  send(method, params = {}, session = this.session) {
    const id = ++this.id;
    this.socket.send(JSON.stringify({ id, method, params, ...(session ? { sessionId: session } : {}) }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }

  /** Open dev.html, with a document if given; waits until the editor shows it. */
  async open(file) {
    const url = `${this.base}dev.html${file ? `?file=${encodeURIComponent(file)}` : ""}`;
    await this.send("Page.navigate", { url });
    await this.waitFor(`document.querySelector(".ProseMirror") && document.querySelector("#status .path")?.textContent?.length > 0`, 15000);
    await this.waitFor(`!document.body.classList.contains("busy")`);
    await sleep(300);
  }

  async eval(expr) {
    const r = await this.send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(`${expr}\n${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`);
    return r.result.value;
  }

  async waitFor(expr, ms = 8000) {
    const end = Date.now() + ms;
    for (;;) {
      if (await this.eval(`!!(${expr})`).catch(() => false)) return;
      if (Date.now() > end) throw new Error(`timed out waiting for: ${expr}`);
      await sleep(60);
    }
  }

  /** Center of the first element matching `sel` (optionally whose text contains `text`). */
  async center(sel, text) {
    const c = await this.eval(`(() => {
      const el = [...document.querySelectorAll(${JSON.stringify(sel)})].find((e) => ${text ? `e.textContent.includes(${JSON.stringify(text)})` : "true"});
      if (!el) return null;
      // Instantly: the page scrolls smoothly, which would move the target after we measured it.
      el.scrollIntoView({ block: "center", behavior: "instant" });
      const r = el.getBoundingClientRect();
      return [r.left + r.width / 2, r.top + r.height / 2];
    })()`);
    if (!c) throw new Error(`no element ${sel}${text ? ` with "${text}"` : ""}`);
    return c;
  }

  async mouse(type, x, y, extra = {}) {
    await this.send("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1, ...extra });
  }

  /** A real click (mouse down / up at the element). */
  async click(sel, text, { clickCount = 1, modifiers = 0 } = {}) {
    const [x, y] = await this.center(sel, text);
    await this.mouse("mouseMoved", x, y);
    await this.mouse("mousePressed", x, y, { clickCount, modifiers });
    await this.mouse("mouseReleased", x, y, { clickCount, modifiers });
    await sleep(120);
  }

  async hover(sel, text) {
    const [x, y] = await this.center(sel, text);
    await this.mouse("mouseMoved", x, y);
    await sleep(120);
  }

  /** Drag from one element to another (e.g. to select table cells). */
  async drag(fromSel, fromText, toSel, toText) {
    const [x1, y1] = await this.center(fromSel, fromText);
    const [x2, y2] = await this.center(toSel, toText);
    await this.mouse("mouseMoved", x1, y1);
    await this.mouse("mousePressed", x1, y1, { buttons: 1 });
    for (let k = 1; k <= 8; k++) {
      await this.mouse("mouseMoved", x1 + ((x2 - x1) * k) / 8, y1 + ((y2 - y1) * k) / 8, { buttons: 1 });
      await sleep(25);
    }
    await this.mouse("mouseReleased", x2, y2, { buttons: 0 });
    await sleep(150);
  }

  /** Press a key, e.g. "a", "Enter", "Escape", "ArrowRight", with "Control" / "Alt" / "Shift". */
  async key(key, ...mods) {
    const bits = { Alt: 1, Control: 2, Meta: 4, Shift: 8 };
    const modifiers = mods.reduce((m, k) => m | bits[k], 0);
    const named = { Enter: [13, "\r"], Escape: [27, ""], End: [35, ""], Home: [36, ""], ArrowRight: [39, ""], ArrowLeft: [37, ""], ArrowDown: [40, ""], ArrowUp: [38, ""], Backspace: [8, ""], Tab: [9, ""] };
    const [code, text] = named[key] ?? [key.toUpperCase().charCodeAt(0), key];
    const base = { key, windowsVirtualKeyCode: code, modifiers, code: key.length === 1 ? `Key${key.toUpperCase()}` : key };
    await this.send("Input.dispatchKeyEvent", { type: text && !modifiers ? "keyDown" : "rawKeyDown", ...base, ...(text && !modifiers ? { text } : {}) });
    await this.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    await sleep(80);
  }

  async type(text) {
    await this.send("Input.insertText", { text });
    await sleep(80);
  }

  /** Wait until no backend call is running. */
  async settle() {
    await sleep(150);
    await this.waitFor(`!document.body.classList.contains("busy")`);
    await sleep(150);
  }

  async screenshot(path, clip) {
    const { data } = await this.send("Page.captureScreenshot", { format: "png", ...(clip ? { clip: { ...clip, scale: 1 } } : {}) });
    const { writeFileSync } = await import("node:fs");
    writeFileSync(path, Buffer.from(data, "base64"));
  }
}
