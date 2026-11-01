import { expect, it } from 'vitest';

// Errors reach people as sentences from a table, never as a raw message or code. The markup of every route may not show
// `{e.message}`, `{err.message}`, `String(e)`, a template `${e}` or `.code}`.
const RULES = [/\{\s*(e|err|error)\.message\s*\}/, /String\(\s*(e|err|error)\s*\)/, /\$\{\s*(e|err|error)\s*\}/, /\.code\s*\}/];
const files = import.meta.glob('../../routes/**/*.svelte', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

// The markup: what is left after the <script> and <style> blocks.
const markup = (src: string) => src.replace(/<script[\s\S]*?<\/script>/g, '').replace(/<style[\s\S]*?<\/style>/g, '');

it('no_raw_errors_in_markup', () => {
  const names = Object.keys(files);
  expect(names).toContain('../../routes/Shell.svelte');
  const hits = names.flatMap((f) =>
    markup(files[f]).split('\n').flatMap((line) => RULES.filter((r) => r.test(line)).map((r) => `${f}: ${line.match(r)![0]}`)),
  );
  expect(hits).toEqual([]);
});

it('the_rules_catch_each_form', () => {
  for (const s of ['{e.message}', '{ err.message }', 'String(e)', '`${e}`', '{res.code}']) expect(RULES.some((r) => r.test(s))).toBe(true);
  expect(RULES.some((r) => r.test('{message}'))).toBe(false);
});
