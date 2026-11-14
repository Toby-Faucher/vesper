import { afterEach, describe, expect, it, vi } from 'vitest';
import { intent, keep, project, type Sample } from './intent';

const fire = (el: EventTarget, type: string, pointerType?: string) => el.dispatchEvent(Object.assign(new Event(type), { pointerType }));

afterEach(() => vi.useRealTimers());

describe('intent', () => {
  it('a resting mouse runs after the delay; leaving first runs nothing', () => {
    vi.useFakeTimers();
    const run = vi.fn();
    const el = new EventTarget();
    intent(run)(el as HTMLElement);
    fire(el, 'pointerenter', 'mouse');
    fire(el, 'pointerleave');
    vi.advanceTimersByTime(200);
    expect(run).not.toHaveBeenCalled();
    fire(el, 'pointerenter', 'mouse');
    vi.advanceTimersByTime(80);
    expect(run).toHaveBeenCalledOnce();
  });

  it('a press or focus runs at once; touch hover does not; cleanup removes the listeners', () => {
    vi.useFakeTimers();
    const run = vi.fn();
    const el = new EventTarget();
    const off = intent(run)(el as HTMLElement);
    fire(el, 'pointerenter', 'touch');
    vi.advanceTimersByTime(200);
    expect(run).not.toHaveBeenCalled();
    fire(el, 'pointerdown', 'touch');
    fire(el, 'focusin');
    expect(run).toHaveBeenCalledTimes(2);
    off();
    fire(el, 'focusin');
    expect(run).toHaveBeenCalledTimes(2);
  });
});

describe('project', () => {
  it('projects a steady move ahead, and ignores drift, a flick and a single sample', () => {
    const at = (x: number, t: number) => ({ x, y: 0, t });
    expect(project([at(0, 0), at(20, 20)])).toEqual({ x: 20 + 120, y: 0 }); // 1 px/ms for 120 ms
    expect(project([at(0, 0), at(2, 20)])).toBeNull(); // 0.1 px/ms
    expect(project([at(0, 0), at(200, 20)])).toBeNull(); // 10 px/ms
    expect(project([at(0, 0)])).toBeNull();
    expect(keep([at(0, 0), at(5, 30)], at(9, 60)).map((s) => s.t)).toEqual([30, 60]); // older than WINDOW dropped
  });
});

// The bench: simulated mouse moves on the sidebar's layout (15 rows of 28 px, 8 to 240 px wide), each a minimum-jerk path
// with a slight arc and 1 px of jitter, timed by Fitts's law, sampled at 60 Hz, then a 120 ms dwell before the click. It runs
// the real `project` and the hover rule (80 ms rest) and measures how long before the click each prefetches the target,
// and how many other rows each fetches. Seeded, so the numbers are stable. BENCH=1 prints the table.
describe('intent bench', () => {
  const ROW = 28, TOP = 100, ROWS = 15, HZ = 1000 / 60, DWELL = 120, HOVER = 80;
  const rowAt = (x: number, y: number) => (x >= 8 && x <= 240 && y >= TOP && y < TOP + ROW * ROWS ? Math.floor((y - TOP) / ROW) : -1);
  let seed = 7;
  const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
  const between = (a: number, b: number) => a + rnd() * (b - a);
  const pick = (target: number) => ({ x: between(40, 210), y: TOP + target * ROW + ROW / 2 + between(-6, 6) });
  const main = () => ({ x: between(300, 1400), y: between(40, 900) });

  type Move = { from: { x: number; y: number }; to: { x: number; y: number }; target: number };
  function run({ from, to, target }: Move) {
    const d = Math.hypot(to.x - from.x, to.y - from.y);
    const dur = 100 + 150 * Math.log2(d / ROW + 1); // Fitts, ms
    const arc = between(-0.08, 0.08) * d;
    const nx = -(to.y - from.y) / d, ny = (to.x - from.x) / d;
    const path: Sample[] = [];
    for (let t = 0; t <= dur; t += HZ) {
      const u = t / dur, s = 10 * u ** 3 - 15 * u ** 4 + 6 * u ** 5, bow = Math.sin(Math.PI * u) * arc;
      path.push({ x: from.x + (to.x - from.x) * s + nx * bow + between(-1, 1), y: from.y + (to.y - from.y) * s + ny * bow + between(-1, 1), t });
    }
    path.push({ ...to, t: dur });
    const click = dur + DWELL;
    // Hover: a row fires HOVER ms after the pointer enters it, if it is still there.
    const hover = new Map<number, number>();
    let cur = -1, since = 0;
    const rowOf = (t: number) => { const p = [...path].reverse().find((q) => q.t <= t)!; return rowAt(p.x, p.y); };
    for (const p of path) {
      const r = rowAt(p.x, p.y);
      if (r !== cur) { cur = r; since = p.t; }
      if (r >= 0 && !hover.has(r) && rowOf(since + HOVER) === r && since + HOVER <= click) hover.set(r, since + HOVER);
    }
    // Prediction: a projected point on a row fires it.
    const pred = new Map<number, number>();
    let s: Sample[] = [];
    for (const p of path) {
      s = keep(s, p);
      const q = project(s);
      const r = q ? rowAt(q.x, q.y) : -1;
      if (r >= 0 && !pred.has(r)) pred.set(r, p.t);
    }
    const lead = (m: Map<number, number>) => Math.max(0, click - (m.get(target) ?? click));
    const both = new Map(hover);
    for (const [r, t] of pred) both.set(r, Math.min(t, both.get(r) ?? Infinity));
    const wrong = (m: Map<number, number>) => [...m.keys()].filter((r) => r !== target).length;
    return { hover: lead(hover), both: lead(both), wrongHover: wrong(hover), wrongBoth: wrong(both) };
  }

  const q = (xs: number[], p: number) => [...xs].sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor(p * xs.length))];
  function bench(name: string, make: () => Move, n = 400) {
    const r = Array.from({ length: n }, () => run(make()));
    const col = (k: keyof (typeof r)[0]) => r.map((x) => x[k]);
    const sum = (xs: number[]) => xs.reduce((a, b) => a + b, 0) / n;
    return { name, hoverMedian: q(col('hover'), 0.5), bothMedian: q(col('both'), 0.5), bothP10: q(col('both'), 0.1),
      wrongHover: +sum(col('wrongHover')).toFixed(2), wrongBoth: +sum(col('wrongBoth')).toFixed(2) };
  }

  it('prediction fetches the room well before hover does, and rarely the wrong one', () => {
    const t = () => Math.floor(between(0, ROWS));
    const rows = [
      bench('main pane to a row', () => { const target = t(); return { from: main(), to: pick(target), target }; }),
      bench('row to another row', () => { const a = t(); let target = t(); if (target === a) target = (a + 3) % ROWS; return { from: pick(a), to: pick(target), target }; }),
      bench('within the main pane', () => ({ from: main(), to: main(), target: -1 })),
    ];
    if (import.meta.env.BENCH) console.table(rows);
    const [toRow, rowToRow, away] = rows;
    expect(toRow.bothMedian).toBeGreaterThan(toRow.hoverMedian + 150);
    expect(toRow.wrongBoth).toBeLessThan(3.5); // a wrong row costs one cached GET per session
    expect(rowToRow.bothMedian).toBeGreaterThanOrEqual(rowToRow.hoverMedian);
    expect(away.wrongBoth).toBeLessThan(0.1);
  });
});
