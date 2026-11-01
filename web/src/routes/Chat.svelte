<script lang="ts">
  // The chat panel for the picked room: the timeline, the search field and the composer. Search and the composer are
  // rebuilt in later tasks (8 and 10).
  import { onMount, untrack } from 'svelte';
  import Timeline from './Timeline.svelte';
  import { ApiFailure, errorText } from '../lib/api';
  import { href } from '../lib/route';
  import { navigate, session } from '../lib/session.svelte';
  import { chat, jumpTo, marks, search, send, typers, upload } from '../lib/chat.svelte';
  import type { Attachment } from '../lib/gen/Attachment';
  import type { Message } from '../lib/gen/Message';
  import type { SearchHit } from '../lib/gen/SearchHit';

  let { roomId }: { roomId: number } = $props();

  const TYPING_EVERY_MS = 3000; // the server takes one typing event per 3 s

  let error = $state('');
  let replyTo = $state<Message | null>(null);
  let composer = $state<HTMLTextAreaElement>();
  let pending = $state<Attachment[]>([]);
  let progress = $state<number | null>(null);
  let sending = $state(false);
  let lastTyping = 0;
  const PASTE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']; // the types the server shows inline
  let now = $state(Date.now());

  let query = $state('');
  let hits = $state<SearchHit[]>([]);
  let searched = $state(false);
  let nextBefore = $state<number | null>(null);
  let searchError = $state('');
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let searchSeq = 0;

  const myId = $derived(session.state.me?.user.user_id ?? '');
  const roomName = (id: number) => session.state.rooms.find((r) => r.id === id)?.name ?? '';
  const name = (id: string) => session.state.users[id]?.display_name ?? 'Someone';
  const typing = $derived(typers(chat.state, roomId, now).filter((id) => id !== myId).map(name));

  onMount(() => {
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => {
      clearInterval(t);
      clearTimeout(searchTimer);
    };
  });

  // A new room: reply and uploads belong to the old one.
  $effect(() => {
    void roomId;
    untrack(() => {
      replyTo = null;
      pending = [];
      error = '';
    });
  });

  /** Runs an action and shows its error. True when it worked. */
  async function act(f: () => Promise<unknown>): Promise<boolean> {
    error = '';
    try {
      await f();
      return true;
    } catch (e) {
      error = errorText(e);
      return false;
    }
  }

  async function submit(e?: Event) {
    e?.preventDefault();
    const room = roomId;
    const body = (chat.drafts[room] ?? '').trim() || pending.map((a) => a.name).join(', '); // a body is required
    if (!body || sending || progress !== null) return;
    sending = true;
    error = '';
    try {
      await send(room, body, replyTo?.id ?? null, pending.map((a) => a.id));
      chat.drafts[room] = '';
      replyTo = null;
      pending = [];
      chat.latest++; // follow your own message, also from an older slice after a jump
    } catch (err) {
      error = err instanceof ApiFailure && err.code === 'conflict' ? 'That file could not be attached. Upload it again.' : errorText(err);
    } finally {
      sending = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void submit();
    }
  }

  function onInput() {
    const t = Date.now();
    if (!chat.drafts[roomId]?.trim() || t - lastTyping < TYPING_EVERY_MS) return;
    lastTyping = t;
    session.sig?.send({ type: 'typing', room_id: roomId });
  }

  async function attach(files: File[]) {
    const room = roomId;
    for (const f of files) {
      error = '';
      progress = 0;
      try {
        const a = await upload(room, f, (p) => (progress = p));
        if (room === roomId) pending = [...pending, a];
      } catch (e) {
        error = e instanceof ApiFailure && e.status === 413 ? 'Files can be up to 25 MB.' : errorText(e);
      } finally {
        progress = null;
      }
    }
  }

  function onPaste(e: ClipboardEvent) {
    const files = [...(e.clipboardData?.files ?? [])].filter((f) => PASTE_TYPES.includes(f.type));
    if (!files.length) return;
    e.preventDefault();
    void attach(files);
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

<section class="flex min-h-0 flex-1 flex-col gap-2 pt-3 pb-4" aria-label="Chat">
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

  <div class="space-y-1 px-4">
  <p class="h-5 text-xs text-fg-3" role="status">
    {typing.length ? `${typing.join(', ')} ${typing.length > 1 ? 'are' : 'is'} typing` : ''}
  </p>
  {#if error}<p class="text-danger" role="alert">{error}</p>{/if}
  <form class="space-y-1" onsubmit={submit}>
    {#if replyTo}
      <p class="text-xs text-fg-3">
        Replying to {name(replyTo.author_id)}
        <button type="button" class="underline" onclick={() => (replyTo = null)}>Cancel reply</button>
      </p>
    {/if}
    {#each pending as a (a.id)}
      <p class="text-xs">
        Attached: {a.name}
        <button type="button" class="underline" onclick={() => (pending = pending.filter((p) => p.id !== a.id))}>Remove</button>
      </p>
    {/each}
    {#if progress !== null}<progress class="w-full" value={progress} max="1" aria-label="Upload progress"></progress>{/if}
    <textarea bind:this={composer} class="w-full rounded bg-element p-2" rows="2" aria-label="Message" placeholder="Message {roomName(roomId)}"
      bind:value={chat.drafts[roomId]} onkeydown={onKey} oninput={onInput} onpaste={onPaste}></textarea>
    <div class="flex items-center gap-2">
      <label class="flex-1 text-sm">
        Attach a file
        <input type="file" class="text-xs" onchange={(e) => { void attach([...(e.currentTarget.files ?? [])]); e.currentTarget.value = ''; }} />
      </label>
      <button class="rounded bg-selected px-3 py-1" disabled={sending || progress !== null}>Send</button>
    </div>
  </form>
  </div>
</section>
