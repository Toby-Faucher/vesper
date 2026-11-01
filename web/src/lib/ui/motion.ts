// Enter and exit motion for popovers, dialogs and the call bar, the unread line fade and the jump flash.
// Durations and easing come from the tokens. Only opacity and transform move; hover and focus stay plain CSS.
import { waapi } from 'animejs/waapi';

type Kind = 'pop' | 'fade' | 'rise';

export const reduced = (): boolean => matchMedia('(prefers-reduced-motion: reduce)').matches;

const token = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
const ms = (name: string) => parseFloat(token(name)) || 0;

// Under reduced motion, pop and rise become a fade at --dur-1.
function frames(kind: Kind): { opacity: [number, number]; transform?: [string, string]; duration: number } {
  if (reduced() || kind === 'fade') return { opacity: [0, 1], duration: ms(reduced() ? '--dur-1' : '--dur-2') };
  if (kind === 'pop') return { opacity: [0, 1], transform: ['scale(0.97)', 'scale(1)'], duration: ms('--dur-2') };
  return { opacity: [0, 1], transform: ['translateY(8px)', 'translateY(0)'], duration: ms('--dur-3') };
}

const rev = <T>([a, b]: [T, T]): [T, T] => [b, a];

export function enter(el: Element, kind: Kind): void {
  waapi.animate(el as HTMLElement, { ...frames(kind), ease: token('--ease-out') });
}

export function exit(el: Element, kind: Kind): Promise<void> {
  const f = frames(kind);
  const a = waapi.animate(el as HTMLElement, {
    opacity: rev(f.opacity),
    ...(f.transform && { transform: rev(f.transform) }),
    duration: f.duration,
    ease: token('--ease-out'),
    persist: true, // hold the end frame until the caller removes the element
  });
  return a.then(() => undefined);
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

// The unread line fading away once read.
export function fadeOut(el: Element): void {
  waapi.animate(el as HTMLElement, { opacity: [1, 0], duration: ms(reduced() ? '--dur-1' : '--dur-3'), ease: token('--ease-out'), persist: true });
}
