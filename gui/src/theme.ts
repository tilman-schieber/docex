// Follow the active Omarchy theme (colors.toml + monospace font), live.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface Theme {
  name?: string | null;
  mode: string;
  colors: Record<string, string>;
  mono?: string | null;
}

// Used when not running on Omarchy.
const FALLBACK_DARK: Record<string, string> = {
  accent: "#7aa2f7",
  selection: "#33467c",
  muted: "#3b4261",
  background: "#1a1b26",
  dark_background: "#16161e",
  darker_background: "#121218",
  lighter_background: "#24283b",
  foreground: "#c0caf5",
  dark_foreground: "#565f89",
  light_foreground: "#9aa5ce",
  red: "#f7768e",
  yellow: "#e0af68",
  orange: "#ff9e64",
  green: "#9ece6a",
  cyan: "#7dcfff",
  blue: "#7aa2f7",
  magenta: "#bb9af7",
};

const VARS: [string, string][] = [
  ["bg", "darker_background"],
  ["chrome", "dark_background"],
  ["page", "background"],
  ["raised", "lighter_background"],
  ["sel", "selection"],
  ["line", "muted"],
  ["text", "foreground"],
  ["dim", "light_foreground"],
  ["faint", "dark_foreground"],
  ["accent", "accent"],
  ["red", "red"],
  ["green", "green"],
  ["yellow", "yellow"],
  ["orange", "orange"],
  ["blue", "blue"],
  ["cyan", "cyan"],
  ["magenta", "magenta"],
];

export let themeName = "";

export function applyTheme(t: Theme) {
  const c = { ...FALLBACK_DARK, ...t.colors };
  const root = document.documentElement;
  for (const [v, key] of VARS) root.style.setProperty(`--${v}`, c[key] || FALLBACK_DARK[key]);
  setAuthorPalette(c);
  root.style.setProperty("--mono", `"${t.mono || "JetBrainsMono Nerd Font"}", "JetBrains Mono", ui-monospace, monospace`);
  root.style.colorScheme = t.mode === "light" ? "light" : "dark";
  root.dataset.mode = t.mode;
  themeName = t.name || "";
}

export async function initTheme(onChange?: () => void) {
  try {
    applyTheme(await invoke<Theme>("theme"));
  } catch {
    applyTheme({ mode: "dark", colors: {} });
  }
  await listen<Theme>("theme-changed", (e) => {
    applyTheme(e.payload);
    onChange?.();
  });
}

// Author colors: the theme's hues (not green/red, which mean insertion and
// deletion), then muted and vivid versions of them at the same lightness, so
// no one stands out by brightness. Colours too close to one already taken are
// skipped. People get them in order of appearance, so the first ones never
// share a colour.
const AUTHOR_HUES = ["blue", "magenta", "orange", "cyan", "yellow", "accent"];
const MIN_DIST = 0.07; // OKLab distance two author colours keep at least

type Lab = [number, number, number];
const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const gam = (c: number) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);

function hexToLab(hex: string): Lab | null {
  const m = /^#?([0-9a-f]{6})/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  const [r, g, b] = [n >> 16, (n >> 8) & 255, n & 255].map((v) => lin(v / 255));
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const mm = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [0.2104542553 * l + 0.793617785 * mm - 0.0040720468 * s, 1.9779984951 * l - 2.428592205 * mm + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * mm - 0.808675766 * s];
}

/** Linear sRGB of an OKLab colour; outside [0,1] when out of gamut. */
function labToRgb([L, a, b]: Lab): number[] {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s];
}

const inGamut = (lab: Lab) => labToRgb(lab).every((v) => v >= -1e-4 && v <= 1 + 1e-4);
const labToHex = (lab: Lab) => "#" + labToRgb(lab).map((v) => Math.round(gam(Math.min(1, Math.max(0, v))) * 255).toString(16).padStart(2, "0")).join("");
const dist = (p: Lab, q: Lab) => Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);

/** `lab` moved to lightness `L` and chroma `C` (same hue), as saturated as the screen can show. */
function shade([L0, a, b]: Lab, dL: number, dC: number): Lab {
  const c0 = Math.hypot(a, b);
  const [ua, ub] = c0 > 1e-4 ? [a / c0, b / c0] : [0, 0];
  const L = L0 + dL;
  let lo = 0, hi = Math.max(0, c0 + dC);
  if (inGamut([L, ua * hi, ub * hi])) return [L, ua * hi, ub * hi];
  for (let i = 0; i < 20; i++) {
    const mid = (lo + hi) / 2;
    if (inGamut([L, ua * mid, ub * mid])) lo = mid;
    else hi = mid;
  }
  return [L, ua * lo, ub * lo];
}

// Variants of each hue, in the order they are handed out: the theme's own,
// more and less saturated at the same lightness, then lighter and darker.
const STEPS: [number, number][] = [[0, 0], [0, 0.08], [0, -0.08], [0.12, 0], [-0.12, 0]];

/** Distinct author colours for a theme. */
export function authorPalette(colors: Record<string, string>): string[] {
  const bases = AUTHOR_HUES.map((k) => hexToLab(colors[k] ?? "")).filter((c): c is Lab => !!c);
  // Variants stay clear of the page background (readable names and avatars)
  // and of the insertion green and deletion red.
  const bg = hexToLab(colors.background ?? "");
  const avoid = ["green", "red"].map((k) => hexToLab(colors[k] ?? "")).filter((c): c is Lab => !!c);
  const out: Lab[] = [];
  for (const [dL, dC] of STEPS) {
    for (const c of bases) {
      const v = shade(c, dL, dC);
      const own = dL === 0 && dC === 0;
      if (!own && ((bg && Math.abs(v[0] - bg[0]) < 0.3) || avoid.some((a) => dist(a, v) < 1.5 * MIN_DIST))) continue;
      // The theme's own colours only give way to near-duplicates (accent = blue).
      if (out.every((o) => dist(o, v) >= (own ? MIN_DIST / 3 : MIN_DIST))) out.push(v);
    }
  }
  return out.map(labToHex);
}

let paletteSize = AUTHOR_HUES.length;
const slots = new Map<string, number>();

function setAuthorPalette(colors: Record<string, string>) {
  const p = authorPalette(colors);
  const root = document.documentElement;
  p.forEach((c, i) => root.style.setProperty(`--au${i}`, c));
  paletteSize = Math.max(1, p.length);
}

/** The people in the document, in order of appearance (you first): each keeps
 *  its colour while it stays in the document, newcomers take the first free one. */
export function assignAuthors(names: Iterable<string>) {
  const want = new Set(names);
  for (const n of [...slots.keys()]) if (!want.has(n)) slots.delete(n);
  const used = new Set(slots.values());
  let next = 0;
  for (const n of want) {
    if (slots.has(n)) continue;
    while (used.has(next)) next++;
    slots.set(n, next);
    used.add(next);
  }
}

export function authorColor(name: string): string {
  let i = slots.get(name);
  if (i === undefined) {
    let h = 0;
    for (const ch of name) h = (h * 31 + ch.codePointAt(0)!) >>> 0;
    i = h;
  }
  return `var(--au${i % paletteSize})`;
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return "?";
  return (parts[0][0] + (parts.length > 1 ? parts[parts.length - 1][0] : "")).toUpperCase();
}
