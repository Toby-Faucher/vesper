// Prefetch on intent. A registered element runs its `run` when the user looks likely to use it: the mouse heading for
// it (pointer velocity, projected ahead), the mouse resting on it for 80 ms, a press (touch included) or keyboard
// focus (focusin, so a wrapper works). `run` must be cheap to call again: it fires on every hit. With `cue`, the element
// the pointer heads for also shows it (`data-intent`, styled by the caller).

export type Sample = { x: number; y: number; t: number };

// The velocity is measured over the last WINDOW ms and projected AHEAD ms. Slower is drift and faster is a flick: neither
// says where the pointer will stop. Tuned with the bench in intent.test.ts (BENCH=1).
export const WINDOW = 50;
export const AHEAD = 120;
export const MIN_SPEED = 0.2; // px/ms
export const MAX_SPEED = 4;

/** `s` with `p` added and samples older than WINDOW dropped. */
export const keep = (s: Sample[], p: Sample): Sample[] => [...s.filter((q) => p.t - q.t <= WINDOW), p];

/** Where the pointer will be AHEAD ms from now at its current velocity, or null when it is not moving with intent (too
 * few samples, drift or a flick). Only the end point: the bench found that checking nearer points first fires the rows
 * crossed on the way and loses the target (lead 297 ms instead of 401). */
export function project(s: Sample[], ahead = AHEAD): { x: number; y: number } | null {
  const a = s[0], b = s.at(-1);
  if (!a || !b || b.t - a.t < 10) return null;
  const vx = (b.x - a.x) / (b.t - a.t), vy = (b.y - a.y) / (b.t - a.t);
  const v = Math.hypot(vx, vy);
  if (v < MIN_SPEED || v > MAX_SPEED) return null;
  return { x: b.x + vx * ahead, y: b.y + vy * ahead };
}

const targets = new WeakMap<Element, { run: () => void; cue: boolean }>();
let samples: Sample[] = [];
let listening = false;

// The cue: an element registered with `cue` gets `data-intent` while the pointer heads for it. One element at a time; a
// miss keeps the mark for CUE_HOLD ms, so a jittery projection does not flicker. Hover, a press or a non-cue hit clears it.
export const CUE_HOLD = 150;
let marked: Element | null = null;
let hold: ReturnType<typeof setTimeout> | undefined;
function mark(el: Element | null): void {
  clearTimeout(hold);
  if (el === marked) return;
  marked?.removeAttribute('data-intent');
  el?.setAttribute('data-intent', '');
  marked = el;
}

function onMove(e: PointerEvent): void {
  if (e.pointerType !== 'mouse') return;
  samples = keep(samples, { x: e.clientX, y: e.clientY, t: e.timeStamp });
  const p = project(samples);
  for (let el = p && document.elementFromPoint(p.x, p.y); el; el = el.parentElement) {
    const t = targets.get(el);
    if (!t) continue;
    if (!t.cue || el.contains(e.target as Node)) mark(null); // already over it: hover shows it
    else {
      mark(el);
      hold = setTimeout(() => mark(null), CUE_HOLD);
    }
    return t.run();
  }
}

export const intent = (run: () => void, { cue = false } = {}) => (el: HTMLElement) => {
  targets.set(el, { run, cue });
  if (!listening && typeof document !== 'undefined') {
    listening = true;
    document.addEventListener('pointermove', onMove, { passive: true });
    document.addEventListener('pointerdown', () => mark(null), true);
  }
  let t: ReturnType<typeof setTimeout> | undefined;
  const now = () => { clearTimeout(t); run(); };
  const enter = (e: PointerEvent) => {
    if (marked === el) mark(null);
    if (e.pointerType === 'mouse') t = setTimeout(now, 80);
  };
  const leave = () => clearTimeout(t);
  const ev: [string, EventListener][] = [['pointerenter', enter as EventListener], ['pointerleave', leave], ['pointerdown', now], ['focusin', now]];
  for (const [k, f] of ev) el.addEventListener(k, f);
  return () => { clearTimeout(t); targets.delete(el); for (const [k, f] of ev) el.removeEventListener(k, f); }; // a mark goes with its hold
};
