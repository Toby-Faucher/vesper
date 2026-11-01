// Where a popover goes next to its anchor: on the asked side, flipped when it does not fit there and does on the other side,
// then clamped 8 px inside the viewport. Coordinates are viewport pixels for a `position: fixed` element.
export type Side = 'bottom' | 'top' | 'right' | 'left';
type Box = { left: number; top: number; right: number; bottom: number };

const M = 8;
const clamp = (v: number, len: number, max: number) => Math.max(M, Math.min(v, max - M - len));

export function place(
  anchor: Box,
  size: { w: number; h: number },
  view: { w: number; h: number },
  side: 'bottom' | 'top' | 'right' = 'bottom',
  gap = 4,
): { x: number; y: number; side: Side } {
  if (side === 'right') {
    const right = anchor.right + gap;
    const left = anchor.left - gap - size.w;
    const flip = right + size.w > view.w - M && left >= M;
    return { x: clamp(flip ? left : right, size.w, view.w), y: clamp(anchor.top, size.h, view.h), side: flip ? 'left' : 'right' };
  }
  const below = anchor.bottom + gap;
  const above = anchor.top - gap - size.h;
  const fitsBelow = below + size.h <= view.h - M;
  const fitsAbove = above >= M;
  const s = side === 'bottom' ? (!fitsBelow && fitsAbove ? 'top' : 'bottom') : !fitsAbove && fitsBelow ? 'bottom' : 'top';
  return { x: clamp(anchor.left, size.w, view.w), y: clamp(s === 'bottom' ? below : above, size.h, view.h), side: s };
}
