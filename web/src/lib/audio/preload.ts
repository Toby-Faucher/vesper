const URLS = ['/dsp/vesper_dsp.js', '/dsp/vesper_dsp_bg.wasm', '/dsp/model.tar.gz'];

let started: Promise<void> | undefined;

/** Test hook: forget that a preload ran. */
export function resetPreload(): void {
  started = undefined;
}

/**
 * Warms the HTTP cache with the denoiser files so the worker's fetch after Join is a quick revalidation.
 * Never throws, runs once, and skips when the user asked to save data.
 */
export function preloadDenoiserAssets(fetchFn: typeof fetch = fetch): Promise<void> {
  if ((navigator as Navigator & { connection?: { saveData?: boolean } }).connection?.saveData) return Promise.resolve();
  started ??= Promise.all(
    URLS.map(async (u) => {
      try {
        const r = await fetchFn(u, { cache: 'default', credentials: 'same-origin' });
        await r.arrayBuffer(); // read the body so the whole file lands in the cache
      } catch {
        /* best effort: the worker fetches it again on Join */
      }
    }),
  ).then(() => undefined);
  return started;
}
