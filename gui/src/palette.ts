// Command palette (Ctrl+P): fuzzy search over commands, headings, people and actions.

export interface Entry {
  label: string;
  /** Group shown on the right: Command, Go to, Person, AI, ... */
  group: string;
  hint?: string;
  keywords?: string;
  run: () => unknown;
}

/** Subsequence match score (higher is better), or -1. */
function score(q: string, text: string): number {
  if (!q) return 0;
  const t = text.toLowerCase();
  const idx = t.indexOf(q);
  if (idx >= 0) return 1000 - idx - t.length / 100 + (idx === 0 || t[idx - 1] === " " ? 200 : 0);
  let s = 0;
  let last = -1;
  for (const ch of q) {
    const j = t.indexOf(ch, last + 1);
    if (j < 0) return -1;
    s += j === last + 1 ? 5 : 1;
    last = j;
  }
  return s - t.length / 100;
}

export function openPalette(entries: Entry[], placeholder = "Type a command, heading or name…") {
  const old = document.getElementById("palette");
  if (old) {
    old.remove();
    return;
  }
  const root = document.createElement("div");
  root.id = "palette";
  const box = document.createElement("div");
  box.className = "pal-box";
  const input = document.createElement("input");
  input.placeholder = placeholder;
  const list = document.createElement("div");
  list.className = "pal-list";
  box.append(input, list);
  root.append(box);
  document.body.append(root);
  let shown: Entry[] = [];
  let sel = 0;

  const close = () => {
    root.remove();
  };

  function render() {
    const q = input.value.trim().toLowerCase();
    shown = entries
      .map((e) => ({ e, s: score(q, `${e.label} ${e.keywords ?? ""} ${e.group}`) }))
      .filter((x) => x.s >= 0)
      .sort((a, b) => b.s - a.s)
      .slice(0, 60)
      .map((x) => x.e);
    if (!q) shown = entries.slice(0, 60);
    sel = Math.min(sel, Math.max(0, shown.length - 1));
    list.replaceChildren(
      ...shown.map((e, k) => {
        const row = document.createElement("div");
        row.className = `pal-row${k === sel ? " sel" : ""}`;
        const l = document.createElement("span");
        l.className = "pal-label";
        l.textContent = e.label;
        const g = document.createElement("span");
        g.className = "pal-group";
        g.textContent = e.hint ? `${e.hint} · ${e.group}` : e.group;
        row.append(l, g);
        row.addEventListener("mousedown", (ev) => {
          ev.preventDefault();
          run(e);
        });
        row.addEventListener("mousemove", () => {
          if (sel !== k) {
            sel = k;
            render();
          }
        });
        return row;
      }),
    );
    if (!shown.length) {
      const empty = document.createElement("div");
      empty.className = "pal-empty";
      empty.textContent = "No match";
      list.append(empty);
    }
    list.querySelector(".sel")?.scrollIntoView({ block: "nearest" });
  }

  function run(e: Entry) {
    close();
    void e.run();
  }

  input.addEventListener("input", () => {
    sel = 0;
    render();
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") {
      sel = Math.min(sel + 1, shown.length - 1);
      render();
    } else if (e.key === "ArrowUp") {
      sel = Math.max(sel - 1, 0);
      render();
    } else if (e.key === "Enter") {
      if (shown[sel]) run(shown[sel]);
    } else if (e.key === "Escape") close();
    else return;
    e.preventDefault();
    e.stopPropagation();
  });
  root.addEventListener("mousedown", (e) => {
    if (e.target === root) close();
  });
  render();
  input.focus();
}
