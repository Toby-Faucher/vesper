// Roving focus in a menu: which item a key moves to, or null when the key is not the menu's.
// Up and Down wrap, Home and End jump, a printable character jumps to the next item starting with it.
export function menuMove(key: string, i: number, labels: string[]): number | null {
  const n = labels.length;
  if (!n) return null;
  if (key === 'ArrowDown') return (i + 1) % n;
  if (key === 'ArrowUp') return i < 0 ? n - 1 : (i - 1 + n) % n;
  if (key === 'Home') return 0;
  if (key === 'End') return n - 1;
  if ([...key].length !== 1 || !key.trim()) return null;
  const k = key.toLocaleLowerCase();
  for (let step = 1; step <= n; step++) {
    const j = (i + step + n) % n;
    if (labels[j].trim().toLocaleLowerCase().startsWith(k)) return j;
  }
  return null;
}
