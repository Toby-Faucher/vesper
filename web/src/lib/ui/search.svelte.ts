import { ApiFailure, errorText } from '../api';
import type { SearchHit } from '../gen/SearchHit';
import type { SearchPage } from '../gen/SearchPage';
import type { SearchQuery } from '../gen/SearchQuery';

export type Query = Omit<SearchQuery, 'before'>;

/**
 * The search dialog's requests. `set` waits `wait` ms after the last change (the server allows 10 searches, then one
 * a second), drops answers to an older query, and on a 429 says "slow" and tries the same page again a second later.
 * Results are newest first; `more` asks for the page before the last hit (the id cursor).
 */
export class Searcher {
  hits = $state<SearchHit[]>([]);
  next = $state<number | null>(null);
  status = $state<'idle' | 'loading' | 'done' | 'slow' | 'error'>('idle');
  error = $state('');
  paging = $state(false);
  #q: Query | null = null;
  #key = '';
  #seq = 0;
  #timer: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private run: (q: SearchQuery) => Promise<SearchPage>,
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

  stop(): void {
    clearTimeout(this.#timer);
    this.#seq++;
  }

  async #fetch(before: number | null): Promise<void> {
    const seq = ++this.#seq;
    try {
      const page = await this.run({ ...this.#q!, before });
      if (seq !== this.#seq) return;
      this.hits = before === null ? page.hits : [...this.hits, ...page.hits];
      [this.next, this.status, this.paging] = [page.next_before, 'done', false];
    } catch (e) {
      if (seq !== this.#seq) return;
      if (e instanceof ApiFailure && e.status === 429) {
        this.status = 'slow';
        this.#timer = setTimeout(() => void this.#fetch(before), 1000);
        return;
      }
      [this.status, this.error, this.paging] = ['error', errorText(e), false];
    }
  }
}
