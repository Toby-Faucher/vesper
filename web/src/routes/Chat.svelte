<script lang="ts">
  // The chat panel for the picked room: the timeline, the composer and the search field. Search is rebuilt in Task 10.
  import { onMount } from 'svelte';
  import Composer from './Composer.svelte';
  import Timeline from './Timeline.svelte';
  import { ApiFailure, errorText } from '../lib/api';
  import { href } from '../lib/route';
  import { navigate, session } from '../lib/session.svelte';
  import { chat, jumpTo, marks, search } from '../lib/chat.svelte';
  import type { Message } from '../lib/gen/Message';
  import type { SearchHit } from '../lib/gen/SearchHit';

  let { roomId }: { roomId: number } = $props();

  let error = $state('');
  let replyTo = $state<Message | null>(null);
  let composer = $state<Composer>();

  let query = $state('');
  let hits = $state<SearchHit[]>([]);
  let searched = $state(false);
  let nextBefore = $state<number | null>(null);
  let searchError = $state('');
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let searchSeq = 0;

  const roomName = (id: number) => session.state.rooms.find((r) => r.id === id)?.name ?? '';
  const name = (id: string) => session.state.users[id]?.display_name ?? 'Someone';

  onMount(() => () => clearTimeout(searchTimer));

  // A new room: an action error belongs to the old one.
  $effect(() => {
    void roomId;
    error = '';
  });

  /** Runs an action and shows its error (`conflict` replaces the table's text for a 409). True when it worked. */
  async function act(f: () => Promise<unknown>, conflict?: string): Promise<boolean> {
    error = '';
    try {
      await f();
      return true;
    } catch (e) {
      error = conflict && e instanceof ApiFailure && e.status === 409 ? conflict : errorText(e);
      return false;
    }
  }

  function onSearchInput() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void runSearch(null), 300); // the server allows about one search a second
  }

  async function runSearch(before: number | null) {
    const q = query.trim();
    const seq = ++searchSeq;
    if (q.length < 2) {
      // one letter is the slow prefix case on the server
      [hits, searched, nextBefore, searchError] = [[], false, null, ''];
      return;
    }
    try {
      const page = await search({ q, room_id: null, author_id: null, from: null, to: null, before });
      if (seq !== searchSeq) return; // an older answer arriving late
      hits = before === null ? page.hits : [...hits, ...page.hits];
      nextBefore = page.next_before;
      searched = true;
      searchError = '';
    } catch (e) {
      if (seq !== searchSeq) return;
      searchError = e instanceof ApiFailure && e.status === 429 ? 'Searching too fast. Wait a moment and try again.' : errorText(e);
    }
  }

  async function jump(h: SearchHit) {
    const { room_id, id } = h.message;
    chat.jump = id; // before the load: the timeline scrolls to it and flashes it as soon as the page lands
    const ok = await act(async () => {
      await jumpTo(room_id, id);
      navigate(href({ name: 'room', id: room_id })); // the shell passes the new room back in as roomId
    });
    if (!ok) chat.jump = null;
  }

  // An empty room invites the first message: focus the composer, unless the reader already moved focus into the pane
  // (a key press landing there must not be taken away when the load finishes).
  function sayHello() {
    const a = document.activeElement;
    if (!a || a === document.body || !a.closest('main')) composer?.focus();
  }

  function reply(m: Message) {
    replyTo = m;
    composer?.focus();
  }
</script>

<section class="flex min-h-0 flex-1 flex-col pt-3" aria-label="Chat">
  <div class="space-y-2 px-4">
  <input type="search" class="w-full rounded bg-element p-2" aria-label="Search messages" placeholder="Search messages"
    bind:value={query} oninput={onSearchInput} />
  {#if searchError}<p class="text-warn" role="alert">{searchError}</p>{/if}
  {#if searched}
    <div class="max-h-48 space-y-1 overflow-y-auto rounded bg-element p-2 text-sm" aria-label="Search results">
      {#each hits as h (h.message.id)}
        <button class="block w-full rounded p-1 text-left hover:bg-hover" onclick={() => jump(h)}>
          <span class="text-fg-2">{name(h.message.author_id)} in {roomName(h.message.room_id)}:</span>
          {#each marks(h.snippet) as part}{#if part.mark}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}
        </button>
      {:else}
        <p class="text-fg-3">No messages found.</p>
      {/each}
      {#if nextBefore !== null}
        <button class="rounded bg-hover px-2" onclick={() => runSearch(nextBefore)}>More results</button>
      {/if}
    </div>
  {/if}
  </div>

  <Timeline {roomId} {act} onreply={reply} onempty={sayHello} />

  {#if error}<p class="shrink-0 px-5 pt-1 text-xs text-danger" role="alert">{error}</p>{/if}
  <Composer {roomId} bind:this={composer} bind:replyTo />
</section>
