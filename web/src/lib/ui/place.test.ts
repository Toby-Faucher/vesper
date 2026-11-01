import { expect, it } from 'vitest';
import { place } from './place';

const rect = (left: number, top: number, w = 32, h = 32) => ({ left, top, right: left + w, bottom: top + h });
const view = { w: 1440, h: 900 };
const size = { w: 200, h: 120 };

it('fits_below', () => {
  expect(place(rect(100, 100), size, view)).toEqual({ x: 100, y: 136, side: 'bottom' });
});

it('flips_above_near_the_bottom_edge', () => {
  expect(place(rect(100, 820), size, view)).toEqual({ x: 100, y: 696, side: 'top' });
  // top asked, no room above: below.
  expect(place(rect(100, 40), size, view, 'top')).toEqual({ x: 100, y: 76, side: 'bottom' });
});

it('clamps_8px_inside_the_viewport', () => {
  expect(place(rect(1400, 100), size, view).x).toBe(1440 - 8 - 200);
  expect(place(rect(-50, 100), size, view).x).toBe(8);
  // Fits neither below nor above: stays on its side, clamped.
  const tall = place(rect(100, 400), { w: 200, h: 700 }, view);
  expect(tall).toEqual({ x: 100, y: 900 - 8 - 700, side: 'bottom' });
});

it('right_flips_to_left', () => {
  expect(place(rect(100, 100), size, view, 'right')).toEqual({ x: 136, y: 100, side: 'right' });
  expect(place(rect(1300, 100), size, view, 'right')).toEqual({ x: 1300 - 4 - 200, y: 100, side: 'left' });
  expect(place(rect(100, 850), size, view, 'right').y).toBe(900 - 8 - 120);
});
