// Prefetch on intent. A registered element runs its `run` when the user looks likely to use it: the mouse heading for
// it (pointer velocity, projected ahead), the mouse resting on it for `delay` ms, a press (touch included) or keyboard
// focus (focusin, so a wrapper works). `run` must be cheap to call again: it fires on every hit.

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

const targets = new WeakMap<Element, () => void>();
let samples: Sample[] = [];
let listening = false;

function onMove(e: PointerEvent): void {
  if (e.pointerType !== 'mouse') return;
  samples = keep(samples, { x: e.clientX, y: e.clientY, t: e.timeStamp });
  const p = project(samples);
  for (let el = p && document.elementFromPoint(p.x, p.y); el; el = el.parentElement) {
    const run = targets.get(el);
    if (run) return run();
  }
}

export const intent = (run: () => void, delay = 80) => (el: HTMLElement) => {
  targets.set(el, run);
  if (!listening && typeof document !== 'undefined') {
    listening = true;
    document.addEventListener('pointermove', onMove, { passive: true });
  }
  let t: ReturnType<typeof setTimeout> | undefined;
  const now = () => { clearTimeout(t); run(); };
  const enter = (e: PointerEvent) => { if (e.pointerType === 'mouse') t = setTimeout(now, delay); };
  const leave = () => clearTimeout(t);
  const ev: [string, EventListener][] = [['pointerenter', enter as EventListener], ['pointerleave', leave], ['pointerdown', now], ['focusin', now]];
  for (const [k, f] of ev) el.addEventListener(k, f);
  return () => { clearTimeout(t); targets.delete(el); for (const [k, f] of ev) el.removeEventListener(k, f); };
};
