import { expect, it } from 'vitest';
import { parse } from 'svelte/compiler';
import src from './CallBar.svelte?raw';
import card from './ShareCard.svelte?raw';
import { shareLine } from "../lib/share/line";

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

// No DOM in the tests, so the share controls are checked in the markup.
it('share_button_is_in_group_calls_only_and_the_card_replaces_it_while_sharing', () => {
  expect(src).toMatch(/\{#if room\?\.kind !== 'dm'\}\s*\{#if shares\.mine\}\s*<ShareCard/);
  expect(src).toMatch(/<IconButton icon=\{monitorUp\} label="Share screen" tip="up"/);
  expect(src).toContain('<ShareDialog');
});
it('share_card_shows_stop_and_the_watchers_outside_any_live_region', () => {
  const parts: Node[] = [];
  walk(parse(card, { modern: true }).fragment, parts);
  expect(card).toContain('>Stop<');
  expect(card).toContain('shareLine(');
  for (const n of parts) expect(attr(n, 'role')).toBeUndefined();
  expect(shareLine({ width: 1920, height: 1080, fps: 60, kbps: 8000, watchers: 2 })).toBe('1920x1080 · 60 fps · 8 Mbps · 2 watching');
});
