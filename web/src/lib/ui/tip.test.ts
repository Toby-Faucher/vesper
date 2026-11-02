import { expect, it, vi } from 'vitest';
import { tipSpot, watchTips } from './tip';

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

it('places_again_when_the_label_changes_while_shown_and_stops_watching_once_gone', () => {
  // Fakes for the few DOM parts watchTips uses: the tip's width follows its label, the button sits at the right edge.
  const handlers: Record<string, (e: Event) => void> = {};
  let changed: MutationCallback = () => {};
  const watched: { el: unknown; filter?: string[] }[] = [];
  class Observer {
    constructor(cb: MutationCallback) { changed = cb; }
    observe(el: unknown, o: MutationObserverInit) { watched.push({ el, filter: o.attributeFilter }); }
    disconnect() { watched.length = 0; }
  }
  const el = {
    dataset: { tip: 'Mute' } as Record<string, string>,
    style: { setProperty: vi.fn() },
    closest: () => el,
    hasAttribute: () => false,
    getBoundingClientRect: () => btn(984, 100),
  };
  vi.stubGlobal('MutationObserver', Observer);
  vi.stubGlobal('document', { addEventListener: (k: string, f: (e: Event) => void) => (handlers[k] = f) });
  vi.stubGlobal('getComputedStyle', () => ({ width: `${el.dataset.tip.length * 8}`, height: '26' }));
  vi.stubGlobal('innerWidth', 1024);
  vi.stubGlobal('innerHeight', 640);
  try {
    watchTips();
    handlers.pointerover({ target: el } as unknown as Event);
    expect(watched).toEqual([{ el, filter: ['data-tip'] }]);
    expect(el.style.setProperty).toHaveBeenLastCalledWith('--tip-dx', '0px'); // "Mute", 32 px: fits
    el.dataset.tip = 'Unmute and undeafen';
    changed([{ target: el } as unknown as MutationRecord], {} as MutationObserver);
    expect(el.style.setProperty).toHaveBeenLastCalledWith('--tip-dx', '-60px'); // 152 px: pulled inside the right edge
    // Leaving (pointer or focus) ends the watch; showing again starts it.
    handlers.pointerout({} as Event);
    expect(watched).toHaveLength(0);
    handlers.focusin({ target: el } as unknown as Event);
    expect(watched).toHaveLength(1);
    handlers.focusout({} as Event);
    expect(watched).toHaveLength(0);
  } finally {
    vi.unstubAllGlobals();
  }
});
