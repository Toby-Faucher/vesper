import png from './favicon-32.png?inline';
// The tab icon in the accent: the 32 px disc and V of public/favicon.ico, recoloured by the logo's matrix on a canvas, as a
// blob: URL (the CSP allows blob: images, not data:). The PNG is inlined in this lazy chunk, so it costs no request.

/** `m` (feColorMatrix values, 0 to 1) on RGBA pixels, in place, like the SVG filter does to the logo. */
export function recolour(px: Uint8ClampedArray, m: number[]): void {
  for (let i = 0; i < px.length; i += 4) {
    const [r, g, b] = [px[i], px[i + 1], px[i + 2]];
    for (let k = 0; k < 3; k++) px[i + k] = m[k * 5] * r + m[k * 5 + 1] * g + m[k * 5 + 2] * b; // the array clamps and rounds
  }
}

let shown = ''; // the blob: URL on the tab now, revoked when replaced
let next: number[] | null | undefined; // the latest wish; quick changes draw only the last one
let busy = false;

/** Shows the icon recoloured by `m`, or the static amber file for null. */
export async function favicon(m: number[] | null): Promise<void> {
  next = m;
  if (busy) return;
  busy = true;
  try {
    while (next !== undefined) {
      const cur = next;
      next = undefined;
      await paint(cur);
    }
  } finally {
    busy = false;
  }
}

async function paint(m: number[] | null): Promise<void> {
  let href = '/favicon.ico';
  if (m) {
    const bytes = Uint8Array.from(atob(png.slice(png.indexOf(',') + 1)), (c) => c.charCodeAt(0));
    const bmp = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
    const canvas = new OffscreenCanvas(32, 32);
    const ctx = canvas.getContext('2d')!;
    ctx.drawImage(bmp, 0, 0);
    const img = ctx.getImageData(0, 0, 32, 32);
    recolour(img.data, m);
    ctx.putImageData(img, 0, 0);
    href = URL.createObjectURL(await canvas.convertToBlob());
  }
  const link = document.querySelector<HTMLLinkElement>('link[rel="icon"]');
  if (link) link.href = href;
  if (shown) URL.revokeObjectURL(shown);
  shown = m ? href : '';
}
