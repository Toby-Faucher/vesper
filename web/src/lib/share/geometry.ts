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

/** Where the picture sits inside a w x h box with object-fit: contain, in px. The reverse of pointIn, for markers. */
export function pictureBox(w: number, h: number, videoW: number, videoH: number): { left: number; top: number; width: number; height: number } {
  const scale = videoW > 0 && videoH > 0 ? Math.min(w / videoW, h / videoH) : 0;
  const pw = scale ? videoW * scale : w, ph = scale ? videoH * scale : h;
  return { left: (w - pw) / 2, top: (h - ph) / 2, width: pw, height: ph };
}

/** The largest 16:9 tiles for n shares in a w x h stage, with at most maxCols columns: the column count and the tile width. */
export function fit(n: number, w: number, h: number, gap: number, maxCols = n): { cols: number; width: number } {
  let best = { cols: 1, width: 0 };
  for (let cols = 1; cols <= Math.max(1, Math.min(n, maxCols)); cols++) {
    const rows = Math.ceil(n / cols);
    const width = Math.min((w - gap * (cols - 1)) / cols, ((h - gap * (rows - 1)) / rows) * (16 / 9));
    if (width > best.width) best = { cols, width };
  }
  return { cols: best.cols, width: Math.max(0, Math.floor(best.width)) };
}
