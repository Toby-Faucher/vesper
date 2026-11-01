import { expect, it } from 'vitest';
import { tipSpot } from './tip';

const view = { w: 1024, h: 640 };
const size = { w: 62, h: 26 };
const btn = (left: number, top: number) => ({ left, top, right: left + 32, bottom: top + 32 });

it('stays_below_and_centred_when_it_fits', () => {
  expect(tipSpot(btn(200, 100), size, view, false, false)).toEqual({ side: 'bottom', dx: 0 });
});

it('flips_above_at_the_bottom_edge', () => {
  // The Settings gear at the bottom of the sidebar at 1024x640: below would end at 670.
  expect(tipSpot(btn(207, 600), size, view, false, false)).toEqual({ side: 'top', dx: 0 });
});

it('keeps_up_and_falls_back_below_at_the_top_edge', () => {
  expect(tipSpot(btn(200, 300), size, view, true, false).side).toBe('top');
  expect(tipSpot(btn(200, 4), size, view, true, false).side).toBe('bottom');
});

it('shifts_inside_the_left_and_right_edges', () => {
  // Centred on a button at x 0..32 the tip would start at -15: moved to 8.
  expect(tipSpot(btn(0, 100), size, view, false, false).dx).toBe(23);
  expect(tipSpot(btn(992, 100), size, view, false, false).dx).toBe(-23);
  // Flush right (data-tip-end) and already inside: no shift.
  expect(tipSpot(btn(900, 100), size, view, false, true)).toEqual({ side: 'bottom', dx: 0 });
});
