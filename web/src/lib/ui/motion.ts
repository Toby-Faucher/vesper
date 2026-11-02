// Every animation in the app goes through this file, as one of the named presets below. anime.js's WAAPI build plays
// them on the compositor; only opacity and transform move (the jump flash fades a background colour). Hover and press
// feedback stay plain CSS on the same tokens.
// Under prefers-reduced-motion every preset is an 80 ms opacity fade (--dur-calm) and the jump flash is an instant change.
import type { waapi as Waapi } from 'animejs/waapi';

// anime is not in the first load: it loads right after, and until it arrives every preset is instant (the first paint
// is calm anyway, see settle). A failed load leaves the app without motion until the next page load.
let waapi: typeof Waapi | undefined;
export const motionReady: Promise<void> = import('animejs/waapi').then((m) => void (waapi = m.waapi), () => {});

export type Preset = 'pop' | 'rise' | 'swap' | 'row' | 'bump';

// Each preset's start pose, its rest pose and its enter duration. An exit runs the poses backwards in --dur-out, ease-in.
const PRESETS: Record<Preset, { move: [string, string]; dur: string; ease?: string }> = {
  pop: { move: ['scale(0.97)', 'scale(1)'], dur: '--dur-2' }, // popovers, menus, dialogs
  // the call bar, a new message, the timeline pills. Under 8 px: a rising last row grows the scroll height, and the timeline
  // still counts a gap under 8 px as at the bottom (timeline.ts follows), so it keeps following.
  rise: { move: ['translateY(6px)', 'translateY(0)'], dur: '--dur-3' },
  swap: { move: ['translateY(4px)', 'translateY(0)'], dur: '--dur-2' }, // another room in the timeline, a settings section
  row: { move: ['translateX(-8px)', 'translateX(0)'], dur: '--dur-2' }, // a person joins or leaves voice
  bump: { move: ['scale(0.6)', 'scale(1)'], dur: '--dur-2', ease: '--ease-pop' }, // a reaction or unread count comes or changes
};

export const reduced = (): boolean => matchMedia('(prefers-reduced-motion: reduce)').matches;

const token = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
// The CSS minifier writes 220ms as .22s, so read the unit.
const ms = (name: string) => { const v = token(name); return (parseFloat(v) || 0) * (v.endsWith('ms') ? 1 : v.endsWith('s') ? 1000 : 1); };

type Frames = { opacity: [number, number]; transform?: [string, string]; duration: number; ease: string };
const rev = <T>([a, b]: [T, T]): [T, T] => [b, a];

function frames(p: Preset, out: boolean): Frames {
  const { move, dur, ease = '--ease-out' } = PRESETS[p];
  const calm = reduced();
  const f: Frames = { opacity: [0, 1], duration: ms(calm ? '--dur-calm' : out ? '--dur-out' : dur), ease: token(out ? '--ease-in' : ease) };
  if (!calm) f.transform = move;
  return out ? { ...f, opacity: rev(f.opacity), ...(f.transform && { transform: rev(f.transform) }) } : f;
}

export function enter(el: Element, p: Preset): void {
  waapi?.animate(el as HTMLElement, frames(p, false));
}

export function exit(el: Element, p: Preset): Promise<void> {
  (el as HTMLElement).inert = true; // leaving: no focus, no click, no second Enter on it
  if (!waapi) return Promise.resolve();
  // persist: hold the end frame until the caller removes the element.
  const a = waapi.animate(el as HTMLElement, { ...frames(p, true), persist: true });
  // anime never completes a persisted animation (its then() would wait forever): wait for the browser's own instead.
  return Promise.all(a.animations.map((x) => x.finished)).then(() => undefined, () => undefined);
}

// An exit that does not hold the element: the real one closes at once, so focus moves on now, and an empty box with its
// fill, border, corners and shadow fades out in its place (dialogs, popovers). Empty, so no text or field is ever in the
// page twice. Call it while `el` is still shown.
export function ghost(el: HTMLElement, p: Preset): void {
  const r = el.getBoundingClientRect();
  if (!r.width || !waapi) return;
  const { backgroundColor, border, borderRadius, boxShadow } = getComputedStyle(el);
  const g = document.createElement('div');
  g.dataset.ghost = '';
  g.setAttribute('aria-hidden', 'true');
  Object.assign(g.style, { position: 'fixed', left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px`, boxSizing: 'border-box', backgroundColor, border, borderRadius, boxShadow, pointerEvents: 'none', zIndex: '50' });
  document.body.append(g);
  void exit(g, p).then(() => g.remove());
}

// No ambient motion while the app fills in: the shell calls settle() on mount, so the rooms, unread counts and people
// that arrive with the first hello appear at once.
// shortcut: a fixed 1.5 s window; on a very slow first load some late rows still animate. Hook it to hello if that shows.
let calmUntil = 0;
export const settle = (ms = 1500): void => void (calmUntil = performance.now() + ms);

// At most this many transitions start in one task: a burst (a reconnect, a fold) shows the rest at once.
const BATCH = 3;
let batch = 0;

// A Svelte transition: in:motion={'row'} out:motion={'row'} (null: none). anime plays the preset; Svelte holds an outgoing element for
// the returned duration, then removes it.
export function motion(el: Element, p: Preset | null, { direction }: { direction: 'in' | 'out' | 'both' }): { duration?: number } {
  // Svelte passes {} for a null parameter.
  if (typeof p !== 'string' || !waapi || performance.now() < calmUntil) return {};
  if (batch++ === 0) setTimeout(() => (batch = 0));
  if (batch > BATCH) return {};
  const out = direction === 'out';
  if (out) void exit(el, p);
  else {
    el.getAnimations().forEach((a) => a.cancel()); // back before its exit ended: drop the held end frame
    enter(el, p);
  }
  return { duration: frames(p, out).duration };
}

// The selection glides: a copy of the old selected row's fill slides to the new one, under the rows' text (the parent
// is `isolate`), while the new row's own fill waits. Both rows share a positioned parent.
export function glide(from: HTMLElement, to: HTMLElement): void {
  const parent = to.offsetParent as HTMLElement | null;
  if (!parent || from.offsetParent !== parent || from === to) return;
  const { borderRadius } = getComputedStyle(to);
  const backgroundColor = 'var(--bg-selected)'; // not the computed one: the row's own colour transition has just begun
  const g = document.createElement('div');
  g.setAttribute('aria-hidden', 'true');
  Object.assign(g.style, { position: 'absolute', zIndex: '-1', left: `${to.offsetLeft}px`, top: `${to.offsetTop}px`, width: `${to.offsetWidth}px`, height: `${to.offsetHeight}px`, borderRadius, backgroundColor, pointerEvents: 'none' });
  parent.append(g);
  to.style.backgroundColor = 'transparent';
  const done = () => { to.style.backgroundColor = ''; g.remove(); };
  if (!waapi || reduced()) return void setTimeout(done);
  const dy = from.offsetTop - to.offsetTop;
  const a = waapi.animate(g, { transform: [`translateY(${dy}px)`, 'translateY(0)'], duration: ms('--dur-2'), ease: token('--ease-out'), persist: true });
  void Promise.all(a.animations.map((x) => x.finished)).then(done, done);
}

// The jump flash: the message background fades in to --accent-subtle, holds and fades back (a fifth of the time each way).
export function flash(el: Element): void {
  const h = el as HTMLElement;
  if (!waapi || reduced()) {
    // An instant change held 600 ms, then removed: no fade.
    h.style.backgroundColor = 'var(--accent-subtle)';
    setTimeout(() => (h.style.backgroundColor = ''), 600);
    return;
  }
  const a = 'var(--accent-subtle)';
  waapi.animate(h, { backgroundColor: ['transparent', a, a, a, a, 'transparent'], duration: 5 * ms('--dur-2'), ease: 'linear' });
}

// The sidebar slides in or out and the main pane follows its edge. The grid already has its final columns (one layout),
// so only transforms move: `side` by its own width (the shell holds it fixed while it leaves) and `main` from where it
// was back to 0. A closing side keeps its end frame until the shell hides it. Reduced motion: no slide at all.
export function slide(side: HTMLElement, main: HTMLElement, open: boolean): Promise<void> {
  for (const e of [side, main]) e.getAnimations().forEach((x) => x.cancel()); // a toggle during the slide starts over
  if (!waapi || reduced()) return Promise.resolve();
  const w = side.offsetWidth;
  const o = { duration: ms('--dur-3'), ease: token('--ease-out') };
  const away = `translateX(${-w}px)`;
  const s = waapi.animate(side, { transform: open ? [away, 'translateX(0)'] : ['translateX(0)', away], ...o, persist: !open });
  const m = waapi.animate(main, { transform: [`translateX(${open ? -w : w}px)`, 'translateX(0)'], ...o });
  return Promise.all([...s.animations, ...m.animations].map((x) => x.finished)).then(() => undefined, () => undefined);
}
