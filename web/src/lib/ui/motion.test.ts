import { afterEach, beforeEach, expect, it, vi } from 'vitest';

// motion.ts in node: matchMedia, the computed tokens and waapi.animate are stubbed; timers are fake.
const animate = vi.hoisted(() => vi.fn(() => ({ animations: [] as { finished: Promise<void> }[] })));
vi.mock('animejs/waapi', () => ({ waapi: { animate } }));
import { enter, exit, flash, ghost, glide, motion, settle } from './motion';
import { follows } from './timeline';

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
  body.length = 0;
  vi.stubGlobal('document', { documentElement: {}, body: { append: (n: Box) => body.push(n) }, createElement: box });
  vi.stubGlobal('getComputedStyle', () => ({ ...LOOK, getPropertyValue: (n: string) => TOKENS[n] ?? '' }));
  vi.stubGlobal('performance', { now: () => now });
});
afterEach(() => {
  vi.runAllTimers(); // ends the task: the batch count starts again
  vi.useRealTimers();
  vi.unstubAllGlobals();
  reduce = false;
});

// A tiny stand-in for the DOM nodes ghost() and glide() make.
type Box = { style: Record<string, string>; dataset: Record<string, string>; attrs: Record<string, string>; children: unknown[]; removed: boolean;
  setAttribute: (k: string, v: string) => void; remove: () => void; append: (n: unknown) => void };
const box = (): Box => {
  const b: Box = { style: {}, dataset: {}, attrs: {}, children: [], removed: false, setAttribute: (k, v) => (b.attrs[k] = v), remove: () => (b.removed = true), append: (n) => b.children.push(n) };
  return b;
};
const body: Box[] = [];
const LOOK = { backgroundColor: 'rgb(1, 2, 3)', border: '1px solid rgb(4, 5, 6)', borderRadius: '10px', boxShadow: 'none' };
// An animation whose browser side finishes when the returned function runs.
const held = () => {
  let done!: () => void;
  const finished = new Promise<void>((r) => (done = r));
  animate.mockReturnValueOnce({ animations: [{ finished }] });
  return async () => { done(); for (let i = 0; i < 5; i++) await Promise.resolve(); };
};
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
  const h = el();
  void exit(h, 'rise');
  expect(h.inert).toBe(true); // leaving: no focus, no click, no second Enter on it
  expect(animate).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({
    opacity: [1, 0], transform: ['translateY(0)', 'translateY(6px)'], duration: 120, ease: TOKENS['--ease-in'],
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

it('a_rising_row_stays_inside_the_follow_margin', () => {
  // The last message rises from below and grows the scroll height for a moment; the timeline must still follow.
  enter(el(), 'rise');
  const [from] = opts()[0].transform as [string, string];
  const px = Number(/translateY\((\d+)px\)/.exec(from)![1]);
  expect(follows(true, px, false)).toBe(true);
});

it('ghost_fades_an_empty_box_with_the_look_of_the_layer_then_removes_it', async () => {
  const finish = held();
  const layer = { getBoundingClientRect: () => ({ left: 10, top: 20, width: 300, height: 200 }) } as unknown as HTMLElement;
  ghost(layer, 'pop');
  expect(body).toHaveLength(1);
  const g = body[0];
  expect(g.children).toHaveLength(0); // no text or field is in the page twice
  expect(g.attrs['aria-hidden']).toBe('true');
  expect(g.dataset.ghost).toBe('');
  expect(g.style).toMatchObject({ position: 'fixed', left: '10px', top: '20px', width: '300px', height: '200px', ...LOOK, pointerEvents: 'none' });
  expect(animate).toHaveBeenCalledWith(g, expect.objectContaining({ opacity: [1, 0], transform: ['scale(1)', 'scale(0.97)'] }));
  expect(g.removed).toBe(false);
  await finish();
  expect(g.removed).toBe(true);
});

it('ghost_skips_a_layer_that_is_not_shown', () => {
  ghost({ getBoundingClientRect: () => ({ left: 0, top: 0, width: 0, height: 0 }) } as unknown as HTMLElement, 'pop');
  expect(body).toHaveLength(0);
  expect(animate).not.toHaveBeenCalled();
});

const row = (top: number, parent: unknown) => ({ offsetParent: parent, offsetTop: top, offsetLeft: 8, offsetWidth: 232, offsetHeight: 28, style: { backgroundColor: '' } }) as unknown as HTMLElement;

it('glide_slides_a_fill_from_the_old_row_to_the_new_one_then_gives_the_row_its_own_back', async () => {
  const finish = held();
  const nav = box();
  const [from, to] = [row(40, nav), row(100, nav)];
  glide(from, to);
  const g = nav.children[0] as Box;
  expect(g.style).toMatchObject({ position: 'absolute', zIndex: '-1', top: '100px', height: '28px', backgroundColor: 'var(--bg-selected)' });
  expect(to.style.backgroundColor).toBe('transparent');
  expect(animate).toHaveBeenCalledWith(g, expect.objectContaining({ transform: ['translateY(-60px)', 'translateY(0)'] }));
  await finish();
  expect(g.removed).toBe(true);
  expect(to.style.backgroundColor).toBe('');
});

it('glide_under_reduced_motion_or_across_parents_does_not_animate', () => {
  reduce = true;
  const nav = box();
  const to = row(100, nav);
  glide(row(40, nav), to);
  glide(row(40, box()), row(100, nav));
  expect(animate).not.toHaveBeenCalled();
  vi.runAllTimers();
  expect(to.style.backgroundColor).toBe('');
  expect((nav.children[0] as Box).removed).toBe(true);
  expect(nav.children).toHaveLength(1);
});
