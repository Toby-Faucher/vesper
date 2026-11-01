import { ApiFailure, errorText } from '../api';
import type { SearchHit } from '../gen/SearchHit';
import type { SearchPage } from '../gen/SearchPage';
import type { SearchQuery } from '../gen/SearchQuery';

export type Query = Omit<SearchQuery, 'before'>;

/**
 * The search dialog's requests. `set` waits `wait` ms after the last change (the server allows 10 searches, then one
 * a second), aborts and drops answers to an older query, and on a 429 tries the same page once more a second later.
 * A second 429 stops at `busy` until `retry` (Enter or the button). Results are newest first; `more` asks for the page
 * before the last hit (the id cursor).
 */
export class Searcher {
  hits = $state<SearchHit[]>([]);
  next = $state<number | null>(null);
  status = $state<'idle' | 'loading' | 'done' | 'busy' | 'error'>('idle');
  error = $state('');
  paging = $state(false);
  #q: Query | null = null;
  #key = '';
  #seq = 0;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #ctl: AbortController | undefined;
  #before: number | null = null; // the page of the last request, for `retry`

  constructor(
    private run: (q: SearchQuery, signal: AbortSignal) => Promise<SearchPage>,
    private wait = 300,
  ) {}

  /** A new query, or null for nothing to ask (an empty box, a hint): clears the results. */
  set(q: Query | null): void {
    const key = q ? JSON.stringify(q) : '';
    if (key === this.#key) return; // a space typed at the end: same query, keep what is shown
    this.stop();
    [this.#q, this.#key, this.paging, this.error] = [q, key, false, ''];
    if (!q) {
      [this.hits, this.next, this.status] = [[], null, 'idle'];
      return;
    }
    this.status = 'loading'; // the old hits stay until the answer, so the list does not blink while typing
    this.#timer = setTimeout(() => void this.#fetch(null), this.wait);
  }

  more(): void {
    if (this.next === null || this.paging || !this.#q) return;
    this.paging = true;
    void this.#fetch(this.next);
  }

  /** After `busy`: the same page again, with one more try a second later. */
  retry(): void {
    if (this.status !== 'busy' || !this.#q) return;
    this.paging = this.#before !== null;
    this.status = this.paging ? 'done' : 'loading';
    void this.#fetch(this.#before);
  }

  /** The rooms you can see changed: hits from rooms not in `rooms` go at once, and the query is asked again. */
  keep(rooms: Set<number>): void {
    this.hits = this.hits.filter((h) => rooms.has(h.message.room_id));
    const q = this.#q;
    this.#key = '';
    this.set(q);
  }

  stop(): void {
    clearTimeout(this.#timer);
    this.#ctl?.abort();
    this.#seq++;
  }

  async #fetch(before: number | null, again = false): Promise<void> {
    const seq = ++this.#seq;
    this.#before = before;
    this.#ctl = new AbortController();
    try {
      const page = await this.run({ ...this.#q!, before }, this.#ctl.signal);
      if (seq !== this.#seq) return;
      this.hits = before === null ? page.hits : [...this.hits, ...page.hits];
      [this.next, this.status, this.paging] = [page.next_before, 'done', false];
    } catch (e) {
      if (seq !== this.#seq) return;
      if (e instanceof ApiFailure && e.status === 429) {
        if (again) [this.status, this.paging] = ['busy', false]; // never a loop: the person asks again
        else this.#timer = setTimeout(() => void this.#fetch(before, true), 1000);
        return;
      }
      [this.status, this.error, this.paging] = ['error', errorText(e), false];
    }
  }
}
