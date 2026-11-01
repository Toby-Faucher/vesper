// Every animation in the app goes through this file, as one of the named presets below. anime.js's WAAPI build plays
// them on the compositor; only opacity and transform move. Hover and press feedback stay plain CSS on the same tokens.
// Under prefers-reduced-motion every preset is an 80 ms opacity fade (--dur-calm) and the jump flash is an instant change.
import { waapi } from 'animejs/waapi';

export type Preset = 'pop' | 'rise' | 'swap' | 'row' | 'bump';

// Each preset's start pose, its rest pose and its enter duration. An exit runs the poses backwards in --dur-out, ease-in.
const PRESETS: Record<Preset, { move: [string, string]; dur: string; ease?: string }> = {
  pop: { move: ['scale(0.97)', 'scale(1)'], dur: '--dur-2' }, // popovers, menus, dialogs
  rise: { move: ['translateY(8px)', 'translateY(0)'], dur: '--dur-3' }, // the call bar, a new message, the timeline pills
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
  waapi.animate(el as HTMLElement, frames(p, false));
}

export function exit(el: Element, p: Preset): Promise<void> {
  // persist: hold the end frame until the caller removes the element.
  const a = waapi.animate(el as HTMLElement, { ...frames(p, true), persist: true });
  // anime never completes a persisted animation (its then() would wait forever): wait for the browser's own instead.
  return Promise.all(a.animations.map((x) => x.finished)).then(() => undefined, () => undefined);
}

// An exit that does not hold the element: an inert copy in the same place fades out over the page, so the real one closes
// at once and focus moves on now (dialogs, popovers). Call it while `el` is still shown.
export function ghost(el: HTMLElement, p: Preset): void {
  const r = el.getBoundingClientRect();
  if (!r.width) return;
  const g = el.cloneNode(true) as HTMLElement;
  const fields = el.querySelectorAll<HTMLInputElement>('input, textarea');
  g.querySelectorAll<HTMLInputElement>('input, textarea').forEach((x, i) => (x.value = fields[i]?.value ?? ''));
  for (const n of [g, ...g.querySelectorAll('[id]')]) n.removeAttribute('id');
  // An open <dialog> that joins the page takes the next Esc (its close watcher): the copy is closed and shown by style.
  for (const n of [g, ...g.querySelectorAll('dialog[open]')]) n.removeAttribute('open');
  g.removeAttribute('popover');
  g.inert = true;
  g.setAttribute('aria-hidden', 'true');
  Object.assign(g.style, { display: 'block', position: 'fixed', inset: 'auto', left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px`, margin: '0', pointerEvents: 'none', zIndex: '50' });
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
  if (typeof p !== 'string' || performance.now() < calmUntil) return {};
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
  if (reduced()) return void setTimeout(done);
  const dy = from.offsetTop - to.offsetTop;
  const a = waapi.animate(g, { transform: [`translateY(${dy}px)`, 'translateY(0)'], duration: ms('--dur-2'), ease: token('--ease-out'), persist: true });
  void Promise.all(a.animations.map((x) => x.finished)).then(done, done);
}

// The jump flash: the message background goes to --accent-subtle and fades back.
export function flash(el: Element): void {
  const h = el as HTMLElement;
  if (reduced()) {
    // An instant change held 600 ms, then removed: no fade.
    h.style.backgroundColor = 'var(--accent-subtle)';
    setTimeout(() => (h.style.backgroundColor = ''), 600);
    return;
  }
  waapi.animate(h, { backgroundColor: ['var(--accent-subtle)', 'transparent'], duration: 600 + ms('--dur-3'), ease: token('--ease-out') });
}
