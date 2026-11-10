export const SPEEDTEST_BYTES = 4 * 1024 * 1024;

const defaultPost = (body: Blob) => fetch('/api/speedtest', { method: 'POST', body, credentials: 'same-origin' });

/** Upload speed in kbps (rounded down), or null if the test failed or was rate limited. */
export async function measureUploadKbps(
  post: (b: Blob) => Promise<Response> = defaultPost,
  now: () => number = () => performance.now(),
): Promise<number | null> {
  try {
    const body = new Blob([new Uint8Array(SPEEDTEST_BYTES)]);
    const t0 = now();
    const res = await post(body);
    const ms = now() - t0;
    if (!res.ok || ms <= 0) return null;
    return Math.floor((SPEEDTEST_BYTES * 8) / ms);
  } catch {
    return null;
  }
}
