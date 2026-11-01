<script lang="ts">
  // The chat panel for the picked room: the timeline, the composer and the search field. Search is rebuilt in Task 10.
  // Files dragged over the panel show the drop overlay and go to the composer's upload tray. Text and links dragged
  // around are left alone; a file dropped anywhere else in the app is ignored instead of opening in the tab.
  import { onMount, untrack } from 'svelte';
  import Composer from './Composer.svelte';
  import Timeline from './Timeline.svelte';
  import { ApiFailure, errorText } from '../lib/api';
  import { href } from '../lib/route';
  import { navigate, session } from '../lib/session.svelte';
  import { chat, jumpTo, marks, search } from '../lib/chat.svelte';
  import type { Message } from '../lib/gen/Message';
  import type { SearchHit } from '../lib/gen/SearchHit';
  import * as I from '../lib/ui/icons';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import { Tray } from '../lib/ui/tray.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';

  let { roomId }: { roomId: number } = $props();

  let error = $state('');
  let replyTo = $state<Message | null>(null);
  let composer = $state<Composer>();
  const tray = new Tray();
  let drag = $state(0); // dragenter minus dragleave over the panel, for file drags only

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

  // A new room: an action error and the files waiting to be sent belong to the old one.
  $effect(() => {
    void roomId;
    error = '';
    untrack(() => tray.clear()); // reads the tray: only the room change may re-run this
  });
  onMount(() => () => tray.clear());

  const hasFiles = (e: DragEvent) => !!e.dataTransfer?.types.includes('Files');
  function onDragOver(e: DragEvent) {
    if (!hasFiles(e)) return;
    e.preventDefault();
    e.dataTransfer!.dropEffect = 'copy';
  }
  function onDrop(e: DragEvent) {
    if (!hasFiles(e)) return;
    e.preventDefault();
    drag = 0;
    tray.add(roomId, [...e.dataTransfer!.files]);
    composer?.focus();
  }

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

<svelte:window ondragover={(e) => hasFiles(e) && e.preventDefault()} ondrop={(e) => { if (hasFiles(e)) { e.preventDefault(); drag = 0; } }}
  ondragend={() => (drag = 0)} />

<section class="relative flex min-h-0 flex-1 flex-col pt-3" aria-label="Chat"
  ondragenter={(e) => { if (hasFiles(e)) { e.preventDefault(); drag++; } }}
  ondragleave={(e) => { if (hasFiles(e)) drag = Math.max(0, drag - 1); }}
  ondragover={onDragOver} ondrop={onDrop}>
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
  <Composer {roomId} {tray} bind:this={composer} bind:replyTo />

  {#if drag}
    <div class="drop" aria-hidden="true"><Icon d={I.paperclip} />Drop to attach</div>
  {/if}
  <span class="sr-only" aria-live="polite">{drag ? UI_MESSAGES.dropFiles : ''}</span>
</section>

<style>
  /* Plain and still: a dashed amber edge over the panel, no motion. It lets drag events through to the panel. */
  .drop {
    position: absolute;
    inset: 8px 16px 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border: 2px dashed var(--accent-line);
    border-radius: var(--radius-lg);
    background: color-mix(in oklab, var(--bg-app) 88%, transparent);
    color: var(--fg);
    font-size: 14px;
    font-weight: 500;
    line-height: 20px;
    pointer-events: none;
  }
</style>
