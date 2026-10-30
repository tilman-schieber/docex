// Mirrors docex_core::docmodel and the view returned by the Tauri backend.

export interface Span {
  t?: string;
  obj?: string;
  alt?: string;
  b?: boolean;
  i?: boolean;
  s?: boolean;
  c?: boolean;
  u?: boolean;
  sup?: boolean;
  sub?: boolean;
  /** Highlighter colour, Word's name ("yellow", "green", ...). */
  hl?: string;
  /** Font colour, "#RRGGBB". */
  color?: string;
  link?: string;
  st?: "ins" | "del";
  author?: string;
  date?: string;
  wid?: string;
  comments?: string[];
}

export interface NewComment {
  start: number;
  end: number;
  text: string;
}

export interface MPara {
  kind: string;
  level: number;
  spans: Span[];
  new_comments?: NewComment[];
  /** Word's automatic number, for display (not sent back). */
  label?: string;
}

export type MBlock =
  | ({ type: "para" } & MPara)
  /** Rows of visible cells; `spans` gives each cell's [colspan, rowspan] when any cell is merged. */
  | { type: "table"; rows: MPara[][][]; spans?: [number, number][][] }
  | { type: "control"; id: number; label: string; paras: MPara[]; locked?: boolean }
  | { type: "opaque"; id: number; label: string; preview: string };

export interface Chart {
  kind: string;
  title: string;
  categories: string[];
  series: { name: string; values: (number | null)[] }[];
}

/** Chart data and SmartArt text by placeholder key. */
export interface Graphics {
  charts: Record<string, Chart>;
  smartart: Record<string, string[]>;
}

/** A header or footer, as text (shown, not edited). */
export interface HeaderFooter {
  footer: boolean;
  which: "default" | "first" | "even";
  lines: string[];
}

export interface MBox {
  id: string;
  after: number | null;
  paras: MPara[];
}

export interface MNote {
  label: string;
  paras: MPara[];
}

export interface DocModel {
  blocks: MBlock[];
  boxes: MBox[];
  notes: MNote[];
  furniture?: HeaderFooter[];
  graphics?: Graphics;
}

export interface Change {
  n: number;
  kind: string;
  author: string;
  date: string;
  text: string;
  part: string;
  wids: string[];
}

export interface Comment {
  id: string;
  author: string;
  date: string;
  text: string;
  parent?: string;
  done: boolean;
  anchor: string;
}

export interface View {
  model: DocModel;
  changes: Change[];
  comments: Comment[];
  track: boolean;
  images: Record<string, string>;
  path: string | null;
  dirty: boolean;
  warnings: string[];
  /** The document's language ("de-DE"), for spell checking. */
  lang?: string | null;
  /** Whether the last review decision can be undone (`undo_review`). */
  can_undo: boolean;
}
