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

// Author colors: theme hues, skipping green/red (insertions/deletions).
const AUTHOR_HUES = ["blue", "magenta", "orange", "cyan", "yellow", "accent"];

export function authorColor(name: string): string {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.codePointAt(0)!) >>> 0;
  return `var(--${AUTHOR_HUES[h % AUTHOR_HUES.length]})`;
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return "?";
  return (parts[0][0] + (parts.length > 1 ? parts[parts.length - 1][0] : "")).toUpperCase();
}
