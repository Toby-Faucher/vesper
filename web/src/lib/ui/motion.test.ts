import { afterEach, beforeEach, expect, it, vi } from 'vitest';

// motion.ts in node: matchMedia, the computed tokens and waapi.animate are stubbed; timers are fake.
const animate = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock('animejs/waapi', () => ({ waapi: { animate } }));
import { enter, exit, flash } from './motion';

let reduce = false;
const TOKENS: Record<string, string> = { '--dur-1': '100ms', '--dur-2': '160ms', '--dur-3': '220ms', '--ease-out': 'cubic-bezier(0.2, 0, 0, 1)' };

beforeEach(() => {
  vi.useFakeTimers();
  animate.mockClear();
  vi.stubGlobal('matchMedia', (q: string) => ({ matches: q === '(prefers-reduced-motion: reduce)' && reduce }));
  vi.stubGlobal('document', { documentElement: {} });
  vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: (n: string) => TOKENS[n] ?? '' }));
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  reduce = false;
});

const el = () => ({ style: { backgroundColor: '' } }) as unknown as HTMLElement;

it('enter_pops_with_scale', () => {
  enter(el(), 'pop');
  expect(animate).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ transform: ['scale(0.97)', 'scale(1)'], duration: 160 }));
});

it('reduced_motion_enter_is_a_short_fade_only', () => {
  reduce = true;
  for (const kind of ['pop', 'rise', 'fade'] as const) enter(el(), kind);
  expect(animate).toHaveBeenCalledTimes(3);
  for (const [, opts] of animate.mock.calls as unknown as [unknown, Record<string, unknown>][]) {
    expect(opts).not.toHaveProperty('transform');
    expect(opts).toMatchObject({ opacity: [0, 1], duration: 100 });
  }
});

it('flash_fades_after_a_600ms_hold', () => {
  flash(el());
  expect(animate).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ duration: 600 + 220 }));
});

it('reduced_motion_flash_holds_600ms_then_clears_without_animating', () => {
  reduce = true;
  const h = el();
  flash(h);
  expect(animate).not.toHaveBeenCalled();
  expect(h.style.backgroundColor).toBe('var(--accent-subtle)');
  vi.advanceTimersByTime(599);
  expect(h.style.backgroundColor).toBe('var(--accent-subtle)');
  vi.advanceTimersByTime(1);
  expect(h.style.backgroundColor).toBe('');
});

it('exit_resolves_when_the_browser_animations_finish', async () => {
  // anime never completes a persisted animation, so exit() must not wait on anime's own then().
  let done!: () => void;
  const finished = new Promise<void>((r) => (done = r));
  animate.mockReturnValueOnce({ animations: [{ finished }], then: () => new Promise(() => {}) } as never);
  let resolved = false;
  const p = exit(el(), 'rise').then(() => (resolved = true));
  await Promise.resolve();
  expect(resolved).toBe(false);
  done();
  await p;
  expect(resolved).toBe(true);
  expect(animate).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ persist: true, opacity: [1, 0] }));
});

it('durations_in_seconds_are_read_as_ms', () => {
  // The production CSS minifier rewrites 220ms as .22s.
  TOKENS['--dur-3'] = '.22s';
  try {
    enter(el(), 'rise');
    expect(animate).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ duration: 220 }));
  } finally {
    TOKENS['--dur-3'] = '220ms';
  }
});
