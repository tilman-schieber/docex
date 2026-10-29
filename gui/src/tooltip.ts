// Hover hints for anything with a `data-tip` attribute ("Label  Shortcut").

let timer: number | undefined;
let shownFor: Element | null = null;

const tip = document.createElement("div");
tip.id = "tip";
tip.hidden = true;
document.body.append(tip);

function hide() {
  clearTimeout(timer);
  tip.hidden = true;
  shownFor = null;
}

function show(el: HTMLElement) {
  const text = el.dataset.tip || "";
  const [label, key] = text.split(/ {2,}/);
  tip.replaceChildren(label);
  if (key) {
    const k = document.createElement("span");
    k.className = "key";
    k.textContent = key;
    tip.append(k);
  }
  tip.hidden = false;
  const r = el.getBoundingClientRect();
  const t = tip.getBoundingClientRect();
  let top = r.bottom + 6;
  if (top + t.height > window.innerHeight - 4) top = r.top - t.height - 6;
  const left = Math.max(4, Math.min(r.left + r.width / 2 - t.width / 2, window.innerWidth - t.width - 4));
  tip.style.top = `${top}px`;
  tip.style.left = `${left}px`;
  shownFor = el;
}

document.addEventListener("mouseover", (e) => {
  const el = (e.target as Element).closest<HTMLElement>("[data-tip]");
  if (el === shownFor) return;
  hide();
  if (!el || !el.dataset.tip) return;
  timer = window.setTimeout(() => show(el), shownFor === null ? 350 : 0);
});
document.addEventListener("mousedown", hide, true);
document.addEventListener("scroll", hide, true);
window.addEventListener("blur", hide);
