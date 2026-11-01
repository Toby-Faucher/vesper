/** `load` once and keep the result; a failed load is forgotten, so the next call tries again (a chunk import offline). */
export function lazy<T>(load: () => Promise<T>): () => Promise<T> {
  let p: Promise<T> | undefined;
  return () => (p ??= load().catch((e) => { p = undefined; throw e; }));
}
