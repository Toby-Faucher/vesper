import { expect, it, vi } from 'vitest';
import { copyText } from './clipboard';

// A stand-in document: what the fallback put in its textarea, and what it ran.
function fakeDoc(exec: () => boolean) {
  const seen = { value: '', commands: [] as string[], removed: false, refocused: false };
  const ta = { value: '', style: {}, setAttribute() {}, select() {}, remove: () => (seen.removed = true) };
  const doc = {
    activeElement: { focus: () => (seen.refocused = true) },
    body: { append: () => {} },
    createElement: () => ta,
    execCommand: (c: string) => {
      seen.commands.push(c);
      seen.value = ta.value;
      return exec();
    },
  } as unknown as Document;
  return { doc, seen };
}

it('uses_the_clipboard_api_when_it_works', async () => {
  const writeText = vi.fn(() => Promise.resolve());
  const { doc, seen } = fakeDoc(() => true);
  expect(await copyText('hi', { clipboard: { writeText } } as never, doc)).toBe(true);
  expect(writeText).toHaveBeenCalledWith('hi');
  expect(seen.commands).toEqual([]);
});

it('falls_back_when_the_api_is_missing_or_refuses', async () => {
  for (const nav of [{}, { clipboard: { writeText: () => Promise.reject(new Error('denied')) } }]) {
    const { doc, seen } = fakeDoc(() => true);
    expect(await copyText('a link', nav as never, doc)).toBe(true);
    expect(seen).toEqual({ value: 'a link', commands: ['copy'], removed: true, refocused: true });
  }
});

it('is_false_when_both_ways_fail', async () => {
  expect(await copyText('x', {} as never, fakeDoc(() => false).doc)).toBe(false);
  expect(await copyText('x', {} as never, fakeDoc(() => { throw new Error('no'); }).doc)).toBe(false);
});
