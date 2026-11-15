// The motion only lazy pages use (Settings, Admin, search, the call bar), kept out of the first load: the sliding
// selectors, the theme reveal and the call bar's morph. The tokens and reduced motion come from motion.ts.
import { glide, ms, reduced, token } from './motion';

// Sliding selectors: {@attach slides} on a list of options (a section nav, a segmented control, a positioned `isolate`
// parent). When the picked option changes (aria-current="page" or aria-pressed="true" moves), its fill glides there.
const PICKED = '[aria-current="page"], [aria-pressed="true"]';
export function slides(node: HTMLElement): () => void {
  let was = node.querySelector<HTMLElement>(PICKED);
  const mo = new MutationObserver(() => {
    const now = node.querySelector<HTMLElement>(PICKED);
    if (was && now && was !== now && was.isConnected) glide(was, now);
    was = now;
  });
  mo.observe(node, { subtree: true, attributes: true, attributeFilter: ['aria-current', 'aria-pressed'] });
  return () => mo.disconnect();
}

// The theme reveal: a new colour scheme spreads as a circle from (x, y), drawn by a view transition (the old page under,
// the new one clipped to a growing circle), --dur-reveal ease-out. No view transitions or reduced motion: at once.
export function reveal(x: number, y: number, change: () => void): void {
  const doc = document as Document & { startViewTransition?: (f: () => void) => { ready: Promise<void> } };
  if (!doc.startViewTransition || reduced()) return change();
  const r = Math.hypot(Math.max(x, innerWidth - x), Math.max(y, innerHeight - y));
  doc.startViewTransition(change).ready.then(() => {
    document.documentElement.animate({ clipPath: [`circle(0px at ${x}px ${y}px)`, `circle(${r}px at ${x}px ${y}px)`] },
      { duration: ms('--dur-reveal'), easing: token('--ease-out'), pseudoElement: '::view-transition-new(root)' });
  }, () => {});
}

// A morph (the call bar to its pill and back): `el` grows or shrinks from the size and corners it had (`from`, measured
// before the change) to its own, --dur-3 ease-out, and its content fades in over --dur-2. Size moves here, not only
// transforms: the bar floats over the pane, so only its own box lays out again. Reduced motion: at once.
export function morph(el: HTMLElement, from: { width: number; height: number; radius: string }): void {
  if (reduced() || !el.animate) return;
  const to = el.getBoundingClientRect();
  const o = { duration: ms('--dur-3'), easing: token('--ease-out') };
  const a = el.animate({ width: [`${from.width}px`, `${to.width}px`], height: [`${from.height}px`, `${to.height}px`], borderRadius: [from.radius, getComputedStyle(el).borderRadius] }, o);
  el.style.overflow = 'clip'; // only while it moves: the bar's tooltips stand out over its edge
  const done = () => void (el.style.overflow = '');
  a.finished.then(done, done);
  for (const c of el.children) c.animate({ opacity: [0, 1] }, { ...o, duration: ms('--dur-2') });
}
