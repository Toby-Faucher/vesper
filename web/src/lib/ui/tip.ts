import { place, type Side } from './place';

// The CSS tooltip ([data-tip] in app.css) sits below its element, above with data-tip-up, or flush right with data-tip-end.
// Near a viewport edge that cuts it off (the Settings gear at the bottom of the sidebar), so before it shows place() picks
// the side and the shift that keep it inside: data-tip-side and --tip-dx on the element.
export function tipSpot(
  r: { left: number; right: number; top: number; bottom: number },
  size: { w: number; h: number },
  view: { w: number; h: number },
  up: boolean,
  end: boolean,
): { side: Side; dx: number } {
  const left = end ? r.right - size.w : (r.left + r.right) / 2 - size.w / 2;
  const p = place({ left, right: left + size.w, top: r.top, bottom: r.bottom }, size, view, up ? 'top' : 'bottom');
  return { side: p.side, dx: p.x - left };
}

function fit(t: HTMLElement) {
  const s = getComputedStyle(t, '::after');
  const spot = tipSpot(t.getBoundingClientRect(), { w: parseFloat(s.width) || 0, h: parseFloat(s.height) || 0 },
    { w: innerWidth, h: innerHeight }, t.hasAttribute('data-tip-up'), t.hasAttribute('data-tip-end'));
  t.dataset.tipSide = spot.side;
  t.style.setProperty('--tip-dx', `${spot.dx}px`);
}

// The tooltip shows on hover or keyboard focus: both events come first. A label that changes while it shows (Mute to
// Unmute) has another width, so the element shown is watched and placed again when its data-tip changes. Leaving it
// ends the watch; moving onto a child leaves and enters again in one go, so the watch comes straight back.
// shortcut: a tip held open by focus stops being watched when the pointer leaves; watch both states if that shows.
export function watchTips(): void {
  const relabel = new MutationObserver((m) => fit(m[0].target as HTMLElement));
  const show = (e: Event) => {
    const t = (e.target as Element | null)?.closest?.<HTMLElement>('[data-tip]');
    if (!t) return;
    relabel.disconnect();
    relabel.observe(t, { attributeFilter: ['data-tip'] });
    fit(t);
  };
  const hide = () => relabel.disconnect();
  document.addEventListener('pointerover', show);
  document.addEventListener('focusin', show);
  document.addEventListener('pointerout', hide);
  document.addEventListener('focusout', hide);
}
