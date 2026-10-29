// Word-level diff (LCS on word / space / punctuation tokens).

export type Op = { type: "eq" | "del" | "ins"; text: string };

export function tokens(s: string): string[] {
  return s.match(/[\p{L}\p{N}_]+|\s+|[^\p{L}\p{N}_\s]/gu) ?? [];
}

/** Diff two strings into equal / deleted / inserted runs of text. */
export function diffWords(a: string, b: string): Op[] {
  const x = tokens(a);
  const y = tokens(b);
  // Trim the common prefix and suffix first: cheap and keeps the table small.
  let pre = 0;
  while (pre < x.length && pre < y.length && x[pre] === y[pre]) pre++;
  let suf = 0;
  while (suf < x.length - pre && suf < y.length - pre && x[x.length - 1 - suf] === y[y.length - 1 - suf]) suf++;
  const xs = x.slice(pre, x.length - suf);
  const ys = y.slice(pre, y.length - suf);
  const ops: Op[] = [];
  const push = (type: Op["type"], text: string) => {
    const last = ops[ops.length - 1];
    if (last && last.type === type) last.text += text;
    else if (text) ops.push({ type, text });
  };
  push("eq", x.slice(0, pre).join(""));
  if (xs.length * ys.length > 4_000_000) {
    // Too big to align: replace the middle wholesale.
    push("del", xs.join(""));
    push("ins", ys.join(""));
  } else {
    const n = xs.length;
    const m = ys.length;
    const L = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
    for (let i = n - 1; i >= 0; i--)
      for (let j = m - 1; j >= 0; j--) L[i][j] = xs[i] === ys[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
    let i = 0;
    let j = 0;
    // Deletions before insertions within a changed stretch, like Word.
    let dels = "";
    let inss = "";
    const flush = () => {
      push("del", dels);
      push("ins", inss);
      dels = inss = "";
    };
    while (i < n || j < m) {
      if (i < n && j < m && xs[i] === ys[j]) {
        flush();
        push("eq", xs[i]);
        i++;
        j++;
      } else if (j < m && (i === n || L[i][j + 1] >= L[i + 1][j])) inss += ys[j++];
      else dels += xs[i++];
    }
    flush();
  }
  push("eq", x.slice(x.length - suf).join(""));
  return ops;
}
