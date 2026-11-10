export type Engine = 'firefox' | 'other';
export const MIN_KBPS = 500, MAX_KBPS = 30_000;

export const engineOf = (userAgent: string): Engine => (/Firefox\/\d/.test(userAgent) ? 'firefox' : 'other');

const blocks = (w: number, h: number) => Math.ceil(w / 16) * Math.ceil(h / 16);

/** Firefox caps the send rate by picture size (16x16 blocks). Other engines have no cap of their own. */
export function ceilingKbps(engine: Engine, width: number, height: number): number {
  if (engine !== 'firefox') return MAX_KBPS;
  const b = blocks(width, height);
  if (b >= blocks(1920, 1200)) return 10_000;
  if (b >= blocks(1280, 720)) return 5_000;
  if (b >= blocks(800, 480)) return 2_500;
  return 2_000;
}

/** About half the measured upload, rounded to 500. Unmeasured: 8 Mbps. Never above the ceiling. */
export function suggestKbps(uploadKbps: number | null, ceiling: number): number {
  const want = uploadKbps === null ? 8_000 : Math.round(uploadKbps / 2 / 500) * 500;
  return Math.min(ceiling, Math.max(MIN_KBPS, want));
}
