/** Map a click to 0..1 coordinates of the picture inside a letterboxed (object-fit: contain) video. Null outside the picture. */
export function pointIn(
  clientX: number, clientY: number,
  box: { left: number; top: number; width: number; height: number },
  videoW: number, videoH: number,
): { x: number; y: number } | null {
  if (videoW <= 0 || videoH <= 0 || box.width <= 0 || box.height <= 0) return null;
  const scale = Math.min(box.width / videoW, box.height / videoH);
  const w = videoW * scale, h = videoH * scale;
  const x = (clientX - (box.left + (box.width - w) / 2)) / w;
  const y = (clientY - (box.top + (box.height - h) / 2)) / h;
  return x < 0 || x > 1 || y < 0 || y > 1 ? null : { x, y };
}
