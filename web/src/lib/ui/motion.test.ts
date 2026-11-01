import { afterEach, beforeEach, expect, it, vi } from 'vitest';

// motion.ts in node: matchMedia, the computed tokens and waapi.animate are stubbed; timers are fake.
const animate = vi.hoisted(() => vi.fn(() => ({ animations: [] as { finished: Promise<void> }[] })));
vi.mock('animejs/waapi', () => ({ waapi: { animate } }));
import { enter, exit, flash, motion, settle } from './motion';

let reduce = false;
const TOKENS: Record<string, string> = {
  '--dur-1': '100ms', '--dur-2': '160ms', '--dur-3': '220ms', '--dur-out': '120ms', '--dur-calm': '80ms',
  '--ease-out': 'cubic-bezier(0.2, 0, 0, 1)', '--ease-in': 'cubic-bezier(0.4, 0, 1, 1)', '--ease-pop': 'cubic-bezier(0.34, 1.56, 0.64, 1)',
};
let now = 10_000;

beforeEach(() => {
  vi.useFakeTimers();
  animate.mockClear();
  vi.stubGlobal('matchMedia', (q: string) => ({ matches: q === '(prefers-reduced-motion: reduce)' && reduce }));
  vi.stubGlobal('document', { documentElement: {} });
  vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: (n: string) => TOKENS[n] ?? '' }));
  vi.stubGlobal('performance', { now: () => now });
});
afterEach(() => {
  vi.runAllTimers(); // ends the task: the batch count starts again
  vi.useRealTimers();
  vi.unstubAllGlobals();
  reduce = false;
});

const cancel = vi.fn();
const el = () => ({ style: { backgroundColor: '' }, getAnimations: () => [{ cancel }] }) as unknown as HTMLElement;
const opts = () => (animate.mock.calls as unknown as [unknown, Record<string, unknown>][]).map(([, o]) => o);

it('enter_pops_with_scale', () => {
  enter(el(), 'pop');
  expect(animate).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({ transform: ['scale(0.97)', 'scale(1)'], duration: 160 }));
});

const PRESETS = ['pop', 'rise', 'swap', 'row', 'bump'] as const;

it('every_preset_enters_in_120_to_220ms_with_ease_out_or_pop_and_moves_only_opacity_and_transform', () => {
  for (const p of PRESETS) enter(el(), p);
  for (const o of opts()) {
    expect(Object.keys(o).sort()).toEqual(expect.arrayContaining(['duration', 'ease', 'opacity']));
    expect(Object.keys(o).every((k) => ['duration', 'ease', 'opacity', 'transform'].includes(k))).toBe(true);
    expect(o.duration).toBeGreaterThanOrEqual(120);
    expect(o.duration).toBeLessThanOrEqual(220);
    expect([TOKENS['--ease-out'], TOKENS['--ease-pop']]).toContain(o.ease);
  }
});

it('exits_run_backwards_in_120ms_with_ease_in', () => {
  void exit(el(), 'rise');
  expect(animate).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({
    opacity: [1, 0], transform: ['translateY(0)', 'translateY(8px)'], duration: 120, ease: TOKENS['--ease-in'],
  }));
});

it('reduced_motion_is_an_80ms_fade_only_for_every_preset_in_and_out', () => {
  reduce = true;
  for (const p of PRESETS) {
    enter(el(), p);
    void exit(el(), p);
  }
  expect(animate).toHaveBeenCalledTimes(PRESETS.length * 2);
  for (const o of opts()) {
    expect(o).not.toHaveProperty('transform');
    expect(o.duration).toBeLessThanOrEqual(80);
  }
});

it('the_transition_plays_the_preset_and_tells_svelte_how_long_to_hold', () => {
  expect(motion(el(), 'row', { direction: 'in' })).toEqual({ duration: 160 });
  expect(motion(el(), 'row', { direction: 'out' })).toEqual({ duration: 120 });
  expect(opts().map((o) => o.opacity)).toEqual([[0, 1], [1, 0]]);
  expect(cancel).toHaveBeenCalled(); // an element back before its exit ended drops the held frame
});

it('the_transition_does_nothing_while_the_app_settles_or_without_a_preset', () => {
  settle(1500);
  expect(motion(el(), 'row', { direction: 'in' })).toEqual({});
  now += 1500;
  expect(motion(el(), null, { direction: 'in' })).toEqual({});
  expect(motion(el(), {} as never, { direction: 'in' })).toEqual({}); // what Svelte passes for null
  expect(animate).not.toHaveBeenCalled();
  expect(motion(el(), 'row', { direction: 'in' })).toEqual({ duration: 160 });
});

it('at_most_three_transitions_start_in_one_task', () => {
  const got = Array.from({ length: 6 }, () => motion(el(), 'row', { direction: 'in' }));
  expect(got.filter((g) => g.duration)).toHaveLength(3);
  expect(animate).toHaveBeenCalledTimes(3);
  vi.advanceTimersByTime(1);
  expect(motion(el(), 'row', { direction: 'in' })).toEqual({ duration: 160 });
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
