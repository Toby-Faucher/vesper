import { expect, it } from 'vitest';

// Colours come only from the roles in styles/tokens.css. Hex, rgb(), hsl(), oklch() and Tailwind palette classes elsewhere fail.
// avatar.ts is the one exception: SLOT_HEX holds the vesper swatch hexes that UserInfo.accent stores, and avatar.test.ts
// checks they equal --avatar-1..8 in tokens.css. accent.ts is the other: the accent presets and the surfaces their contrast
// is checked on, which accent.test.ts checks against tokens.css.
const RULES = [
  /#[0-9a-fA-F]{3,8}\b/,
  /\brgba?\(/,
  /\bhsla?\(/,
  /\boklch\(/,
  /\b(neutral|zinc|gray|slate|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|pink|rose)-\d{2,3}\b/,
];
const files = import.meta.glob(['../../**/*.svelte', '../../**/*.ts', '../../app.css', '!../../**/*.test.ts', '!../../styles/**', '!../gen/**', '!./avatar.ts', '!./accent.ts'], {
  query: '?raw', import: 'default', eager: true,
}) as Record<string, string>;

it('no_raw_colours', () => {
  const names = Object.keys(files);
  expect(names).toContain('../../routes/Shell.svelte');
  expect(names).toContain('../../main.ts');
  expect(names).toContain('../../app.css');
  const hits = names.flatMap((f) =>
    files[f].split('\n').flatMap((line, i) => RULES.filter((r) => r.test(line)).map((r) => `${f}:${i + 1}: ${line.match(r)![0]}`)),
  );
  expect(hits).toEqual([]);
});
