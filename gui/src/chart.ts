// Charts and SmartArt from the document, drawn for reading: Word's cached
// chart data as a small SVG chart, a SmartArt diagram as its text.

import type { Chart, Graphics } from "./types";

/** Chart data and SmartArt text by placeholder key (`<!--docex:chart rId5-->`). */
export const graphics: Graphics = { charts: {}, smartart: {} };

export function setGraphics(g: Graphics | undefined) {
  graphics.charts = g?.charts ?? {};
  graphics.smartart = g?.smartart ?? {};
}

const COLORS = ["var(--blue)", "var(--orange)", "var(--green)", "var(--magenta)", "var(--cyan)", "var(--yellow)", "var(--red)"];
const SVG = "http://www.w3.org/2000/svg";

function el<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number>, tip?: string): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  if (tip) {
    const t = document.createElementNS(SVG, "title");
    t.textContent = tip;
    e.append(t);
  }
  return e;
}

const fmt = (v: number) => (Number.isInteger(v) ? v.toLocaleString() : v.toLocaleString(undefined, { maximumFractionDigits: 2 }));

/** An upper bound for an axis of four round steps (0, 6, 12, 18, 24). */
function nice(v: number): number {
  if (v <= 0) return 4;
  const step = v / 4;
  const p = 10 ** Math.floor(Math.log10(step));
  return ([1, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10].find((m) => m * p >= step) ?? 10) * p * 4;
}

const W = 520;
const H = 230;
const PAD = { l: 44, r: 10, t: 10, b: 28 };

function axes(svg: SVGSVGElement, lo: number, hi: number, horizontal: boolean) {
  for (let k = 0; k <= 4; k++) {
    const v = lo + ((hi - lo) * k) / 4;
    if (horizontal) {
      const x = PAD.l + ((W - PAD.l - PAD.r) * k) / 4;
      svg.append(el("line", { x1: x, x2: x, y1: PAD.t, y2: H - PAD.b, class: "grid" }));
      const t = el("text", { x, y: H - 8, "text-anchor": "middle", class: "tick" });
      t.textContent = fmt(v);
      svg.append(t);
    } else {
      const y = H - PAD.b - ((H - PAD.t - PAD.b) * k) / 4;
      svg.append(el("line", { x1: PAD.l, x2: W - PAD.r, y1: y, y2: y, class: "grid" }));
      const t = el("text", { x: PAD.l - 6, y: y + 4, "text-anchor": "end", class: "tick" });
      t.textContent = fmt(v);
      svg.append(t);
    }
  }
}

function barsOrLines(c: Chart, svg: SVGSVGElement) {
  const all = c.series.flatMap((s) => s.values.filter((v): v is number => v !== null));
  const lo = Math.min(0, ...all) < 0 ? -nice(-Math.min(...all)) : 0;
  const hi = nice(Math.max(0, ...all));
  const n = Math.max(1, c.categories.length);
  const horizontal = c.kind === "bar";
  axes(svg, lo, hi, horizontal);
  const span = horizontal ? H - PAD.t - PAD.b : W - PAD.l - PAD.r;
  const len = horizontal ? W - PAD.l - PAD.r : H - PAD.t - PAD.b;
  const pos = (v: number) => ((v - lo) / (hi - lo)) * len;
  const slot = span / n;
  c.categories.forEach((cat, i) => {
    const t = horizontal
      ? el("text", { x: PAD.l - 6, y: PAD.t + slot * (i + 0.5) + 4, "text-anchor": "end", class: "tick" })
      : el("text", { x: PAD.l + slot * (i + 0.5), y: H - 10, "text-anchor": "middle", class: "tick" });
    t.textContent = cat.length > 14 ? cat.slice(0, 13) + "…" : cat;
    if (!horizontal || c.series.length) svg.append(t);
  });
  if (c.kind === "line" || c.kind === "area" || c.kind === "scatter" || c.kind === "other") {
    c.series.forEach((s, si) => {
      const pts = s.values.map((v, i) => (v === null ? null : [PAD.l + slot * (i + 0.5), H - PAD.b - pos(v)] as const));
      const d = pts.filter((p) => p).map((p, k) => `${k ? "L" : "M"}${p![0]},${p![1]}`).join(" ");
      if (c.kind === "area" && d) {
        const first = pts.find((p) => p)!;
        const last = [...pts].reverse().find((p) => p)!;
        svg.append(el("path", { d: `${d} L${last[0]},${H - PAD.b - pos(0)} L${first[0]},${H - PAD.b - pos(0)} Z`, style: `fill:${COLORS[si % COLORS.length]};opacity:.25` }));
      }
      svg.append(el("path", { d, style: `fill:none;stroke:${COLORS[si % COLORS.length]};stroke-width:2` }));
      pts.forEach((p, i) => p && svg.append(el("circle", { cx: p[0], cy: p[1], r: 3, style: `fill:${COLORS[si % COLORS.length]}` }, `${s.name} · ${c.categories[i]}: ${fmt(s.values[i]!)}`)));
    });
    return;
  }
  const k = Math.max(1, c.series.length);
  const bw = (slot * 0.8) / k;
  c.series.forEach((s, si) => {
    s.values.forEach((v, i) => {
      if (v === null) return;
      const a = Math.min(pos(v), pos(0));
      const b = Math.abs(pos(v) - pos(0));
      const off = slot * (i + 0.1) + bw * si;
      const tip = `${s.name || "value"} · ${c.categories[i]}: ${fmt(v)}`;
      const r = horizontal
        ? el("rect", { x: PAD.l + a, y: PAD.t + off, width: b, height: bw * 0.92 }, tip)
        : el("rect", { x: PAD.l + off, y: H - PAD.b - a - b, width: bw * 0.92, height: b }, tip);
      r.setAttribute("style", `fill:${COLORS[si % COLORS.length]}`);
      svg.append(r);
    });
  });
}

function pie(c: Chart, svg: SVGSVGElement) {
  const vals = (c.series[0]?.values ?? []).map((v) => Math.max(0, v ?? 0));
  const total = vals.reduce((a, b) => a + b, 0) || 1;
  const cx = W / 2;
  const cy = H / 2;
  const r = H / 2 - 12;
  let a0 = -Math.PI / 2;
  vals.forEach((v, i) => {
    const a1 = a0 + (v / total) * Math.PI * 2;
    const large = a1 - a0 > Math.PI ? 1 : 0;
    const p = (a: number) => `${cx + r * Math.cos(a)},${cy + r * Math.sin(a)}`;
    const d = v >= total ? `M${cx - r},${cy} a${r},${r} 0 1,0 ${2 * r},0 a${r},${r} 0 1,0 ${-2 * r},0` : `M${cx},${cy} L${p(a0)} A${r},${r} 0 ${large} 1 ${p(a1)} Z`;
    svg.append(el("path", { d, style: `fill:${COLORS[i % COLORS.length]};stroke:var(--page);stroke-width:1` }, `${c.categories[i]}: ${fmt(v)} (${Math.round((v / total) * 100)}%)`));
    a0 = a1;
  });
  if (c.kind === "doughnut") svg.append(el("circle", { cx, cy, r: r * 0.55, style: "fill:var(--page)" }));
}

/** A chart for the page: title, SVG, legend. Hover a bar or point for its value. */
export function chartElement(key: string): HTMLElement | null {
  const c = graphics.charts[key];
  if (!c) return null;
  const box = document.createElement("span");
  box.className = "chart";
  box.contentEditable = "false";
  if (c.title) {
    const t = document.createElement("span");
    t.className = "chart-title";
    t.textContent = c.title;
    box.append(t);
  }
  const svg = el("svg", { viewBox: `0 0 ${W} ${H}`, role: "img", "aria-label": c.title || `${c.kind} chart` });
  if (c.kind === "pie" || c.kind === "doughnut") pie(c, svg);
  else barsOrLines(c, svg);
  box.append(svg);
  const legend = document.createElement("span");
  legend.className = "chart-legend";
  const names = c.kind === "pie" || c.kind === "doughnut" ? c.categories : c.series.map((s) => s.name || "values");
  names.forEach((n, i) => {
    const item = document.createElement("span");
    item.style.setProperty("--c", COLORS[i % COLORS.length]);
    item.textContent = n;
    legend.append(item);
  });
  box.append(legend);
  box.title = "Chart from the document (data as cached by Word; edit in Word)";
  return box;
}

/** A SmartArt diagram as its steps. */
export function smartartElement(key: string): HTMLElement | null {
  const items = graphics.smartart[key];
  if (!items) return null;
  const box = document.createElement("span");
  box.className = "smartart";
  box.contentEditable = "false";
  box.title = "SmartArt from the document (edit in Word)";
  items.forEach((t, i) => {
    if (i) {
      const a = document.createElement("i");
      a.textContent = "→";
      box.append(a);
    }
    const s = document.createElement("span");
    s.textContent = t;
    box.append(s);
  });
  return box;
}
