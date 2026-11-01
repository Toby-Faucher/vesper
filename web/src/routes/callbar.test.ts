import { expect, it } from 'vitest';
import { parse } from 'svelte/compiler';
import src from './CallBar.svelte?raw';

// The bar's live regions may only hold words that change on a state change. The connection figure (changes every 2 s
// poll) and the push-to-talk pill (changes on every key press) would be read out over and over.
type Node = { type: string; name?: string; attributes?: { name: string; value: unknown }[]; [k: string]: unknown };
const attr = (n: Node, name: string) => n.attributes?.find((a) => a.name === name)?.value;
const isStatic = (v: unknown, text: string) => Array.isArray(v) && v.length === 1 && v[0].type === 'Text' && v[0].data === text;
function walk(n: unknown, out: Node[]) {
  if (!n || typeof n !== 'object') return;
  if (Array.isArray(n)) return n.forEach((c) => walk(c, out));
  const node = n as Node;
  if (node.type === 'RegularElement') out.push(node);
  for (const [k, v] of Object.entries(node)) if (k !== 'attributes') walk(v, out);
}
const elements: Node[] = [];
walk(parse(src, { modern: true }).fragment, elements);
const used = (n: Node) => JSON.stringify(n.fragment).match(/"name":"(\w+)"/g)?.map((m) => m.slice(8, -1)) ?? [];

it('live_regions_hold_words_only', () => {
  const live = elements.filter((n) => isStatic(attr(n, 'role'), 'status'));
  expect(live.length).toBeGreaterThan(0);
  for (const n of live) {
    expect(used(n)).not.toContain('qFigure');
    expect(used(n)).not.toContain('qText');
  }
  expect(live.some((n) => used(n).includes('qWords'))).toBe(true);
  const ptt = elements.filter((n) => JSON.stringify(attr(n, 'class') ?? '').includes('ptt '));
  expect(ptt).toHaveLength(1);
  // Neither the pill nor anything inside it is live.
  const inside: Node[] = [];
  walk(ptt[0].fragment, inside);
  for (const n of [ptt[0], ...inside]) expect(attr(n, 'role')).toBeUndefined();
});
