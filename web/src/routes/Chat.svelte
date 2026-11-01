<script lang="ts">
  // The chat panel for the picked room. Rough on purpose: a later milestone redesigns it.
  import { onMount, tick, untrack } from 'svelte';
  import Markdown from './Markdown.svelte';
  import { parse } from '../lib/markdown';
  import { ApiFailure, errorText } from '../lib/api';
  import { session } from '../lib/session.svelte';
  import {
    anchorTop, chat, edit, jumpTo, loadNewer, loadOlder, markRead, marks, openRoom, pin, react, remove, search, send, typers, upload,
  } from '../lib/chat.svelte';
  import type { Attachment } from '../lib/gen/Attachment';
  import type { Message } from '../lib/gen/Message';
  import type { SearchHit } from '../lib/gen/SearchHit';

  let { roomId = $bindable() }: { roomId: number } = $props();

  const EMOJIS = ['👍', '😂', '❤️'];
  const TYPING_EVERY_MS = 3000; // the server takes one typing event per 3 s

  let list = $state<HTMLElement>();
  let stick = true; // scrolled to the bottom: follow new messages
  let jumpTarget: number | null = null;
  let error = $state('');
  let replyTo = $state<Message | null>(null);
  let editingId = $state<number | null>(null);
  let editText = $state('');
  let pending = $state<Attachment[]>([]);
  let progress = $state<number | null>(null);
  let sending = $state(false);
  let loadingOlder = false;
  let lastTyping = 0;
  const readSent: Record<number, number> = {};
  const PASTE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']; // the types the server shows inline
  let announce = $state(''); // the newest message from someone else, for screen readers
  let lastId: number | null = null; // newest id shown; null right after a room switch, so its history is not announced
  let now = $state(Date.now());

  let query = $state('');
  let hits = $state<SearchHit[]>([]);
  let searched = $state(false);
  let nextBefore = $state<number | null>(null);
  let searchError = $state('');
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let searchSeq = 0;

  const log = $derived(chat.state.rooms[roomId]);
  const myId = $derived(session.state.me?.user.user_id ?? '');
  const isMod = $derived(session.state.me?.user.role === 'owner' || session.state.me?.user.role === 'moderator');
  const roomName = (id: number) => session.state.rooms.find((r) => r.id === id)?.name ?? '';
  const name = (id: string) => session.state.users[id]?.display_name ?? 'Someone';
  const typing = $derived(typers(chat.state, roomId, now).filter((id) => id !== myId).map(name));
  const time = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const size = (n: number) => (n < 1024 * 1024 ? `${Math.ceil(n / 1024)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`);
  const reacted = (m: Message, e: string) => m.reactions.find((r) => r.emoji === e)?.user_ids ?? [];
  const emojis = (m: Message) => [...EMOJIS, ...m.reactions.map((r) => r.emoji).filter((e) => !EMOJIS.includes(e))];

  onMount(() => {
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => {
      clearInterval(t);
      clearTimeout(searchTimer);
      chat.active = null;
    };
  });

  // A new room: show its newest page, scrolled to the bottom. Reply, edit and uploads belong to the old room.
  $effect(() => {
    const room = roomId;
    untrack(() => {
      stick = jumpTarget === null;
      lastId = null;
      replyTo = null;
      editingId = null;
      pending = [];
      error = '';
      openRoom(room, jumpTarget !== null).catch((e) => (error = errorText(e)));
    });
  });

  // After the list changed: scroll to a jump target, or follow the bottom, then mark read.
  $effect(() => {
    void log?.messages;
    untrack(() => {
      if (jumpTarget !== null) {
        document.getElementById(`m${jumpTarget}`)?.scrollIntoView({ block: 'center' });
        jumpTarget = null;
        stick = false;
      } else if (stick) toBottom();
      const last = log?.messages.at(-1);
      if (last && lastId !== null && last.id > lastId && last.author_id !== myId && !last.deleted) {
        announce = `${name(last.author_id)}: ${last.body}`;
      }
      if (last) lastId = last.id;
      maybeRead();
    });
  });

  function toBottom() {
    if (list) list.scrollTop = list.scrollHeight;
  }
  const atBottom = (el: HTMLElement) => el.scrollHeight - el.scrollTop - el.clientHeight < 8;

  // Mark read only what the reader can see: the newest page, scrolled to the bottom, in a visible tab.
  function maybeRead() {
    const last = log?.messages.at(-1);
    if (!last || log?.moreAfter || !stick || document.visibilityState !== 'visible') return;
    if ((session.state.unread[roomId]?.last_read_id ?? 0) >= last.id || (readSent[roomId] ?? 0) >= last.id) return;
    const room = roomId;
    readSent[room] = last.id;
    markRead(room, last.id).catch(() => (readSent[room] = 0));
  }

  async function older() {
    if (loadingOlder || !list) return;
    loadingOlder = true;
    const [top, height] = [list.scrollTop, list.scrollHeight];
    try {
      await loadOlder(roomId);
      await tick();
      list.scrollTop = anchorTop(top, height, list.scrollHeight);
    } catch (e) {
      error = errorText(e);
    } finally {
      loadingOlder = false;
    }
  }

  function onScroll() {
    if (!list) return;
    stick = atBottom(list);
    if (list.scrollTop < 40 && log?.moreBefore) void older();
    maybeRead();
  }

  async function act(f: () => Promise<unknown>) {
    error = '';
    try {
      await f();
    } catch (e) {
      error = errorText(e);
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
      stick = true;
      toBottom();
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

  async function saveEdit() {
    const id = editingId;
    if (id === null || !editText.trim()) return;
    await act(async () => {
      await edit(id, editText);
      editingId = null;
    });
  }

  function onEditKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      editingId = null;
    } else if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void saveEdit();
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
    jumpTarget = id; // before the load: the list effect runs as soon as the page lands
    await act(async () => {
      await jumpTo(room_id, id);
      roomId = room_id;
    });
    if (error) jumpTarget = null;
  }
</script>

<svelte:document onvisibilitychange={maybeRead} />

<section class="space-y-2" aria-label="Chat">
  <input type="search" class="w-full rounded bg-element p-2" aria-label="Search messages" placeholder="Search messages"
    bind:value={query} oninput={onSearchInput} />
  {#if searchError}<p class="text-warn" role="alert">{searchError}</p>{/if}
  {#if searched}
    <div class="max-h-48 space-y-1 overflow-y-auto rounded bg-element p-2 text-sm" aria-label="Search results">
      {#each hits as h (h.message.id)}
        <button class="block w-full rounded p-1 text-left hover:bg-hover" onclick={() => jump(h)}>
          <span class="text-fg-2">{name(h.message.author_id)} in #{roomName(h.message.room_id)}:</span>
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

  <div bind:this={list} onscroll={onScroll} class="h-[28rem] space-y-2 overflow-y-auto rounded bg-element p-2">
    {#if log?.moreBefore}
      <button class="rounded bg-hover px-2 text-sm" onclick={older}>Load older messages</button>
    {/if}
    {#each log?.messages ?? [] as m (m.id)}
      <article id="m{m.id}" class="rounded p-1 hover:bg-hover">
        <p class="text-xs text-fg-2">
          <span class="font-medium text-fg">{name(m.author_id)}</span>
          {time(m.created_at)}{m.edited_at && !m.deleted ? ' (edited)' : ''}{m.pinned ? ' (pinned)' : ''}
        </p>
        {#if m.reply_to}
          <p class="border-l-2 border-line pl-2 text-xs text-fg-2">
            Reply to {name(m.reply_to.author_id)}: {m.reply_to.excerpt || 'a deleted message'}
          </p>
        {/if}
        {#if m.deleted}
          <p class="text-fg-2 italic">Message deleted</p>
        {:else if editingId === m.id}
          <textarea class="w-full rounded bg-hover p-1" aria-label="Edit message" bind:value={editText}
            onkeydown={onEditKey} {@attach (el) => el.focus()}></textarea>
          <button class="rounded bg-selected px-2 text-sm" onclick={saveEdit}>Save</button>
          <button class="px-2 text-sm" onclick={() => (editingId = null)}>Cancel</button>
        {:else}
          <Markdown nodes={parse(m.body)} />
          {#each m.attachments as a (a.id)}
            {#if a.image}
              <img src="/api/uploads/{a.id}" alt={a.name} class="max-h-64 rounded" onload={() => stick && toBottom()} />
            {:else}
              <a href="/api/uploads/{a.id}" download={a.name} class="block text-fg underline">{a.name} ({size(a.size)})</a>
            {/if}
          {/each}
          <div class="mt-1 flex flex-wrap gap-1 text-xs">
            {#each emojis(m) as e (e)}
              {@const by = reacted(m, e)}
              <button class="rounded px-1 {by.includes(myId) ? 'bg-accent-subtle' : 'bg-hover'}" aria-pressed={by.includes(myId)}
                aria-label="React {e}{by.length ? `, ${by.length}` : ''}" onclick={() => act(() => react(m.id, e, !by.includes(myId)))}>{e}{by.length ? ` ${by.length}` : ''}</button>
            {/each}
            <button class="rounded bg-hover px-1" onclick={() => (replyTo = m)}>Reply</button>
            <button class="rounded bg-hover px-1" onclick={() => act(() => pin(m.id, !m.pinned))}>{m.pinned ? 'Unpin' : 'Pin'}</button>
            {#if m.author_id === myId}
              <button class="rounded bg-hover px-1" onclick={() => ([editingId, editText] = [m.id, m.body])}>Edit</button>
            {/if}
            {#if m.author_id === myId || isMod}
              <button class="rounded bg-hover px-1" onclick={() => confirm('Delete this message?') && act(() => remove(m.id))}>Delete</button>
            {/if}
          </div>
        {/if}
      </article>
    {/each}
    {#if log?.moreAfter}
      <button class="rounded bg-hover px-2 text-sm" onclick={() => act(() => loadNewer(roomId))}>Load newer messages</button>
    {/if}
  </div>

  <p class="h-5 text-xs text-fg-3" role="status">
    {typing.length ? `${typing.join(', ')} ${typing.length > 1 ? 'are' : 'is'} typing` : ''}
  </p>
  <p class="sr-only" aria-live="polite">{announce}</p>
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
    <textarea class="w-full rounded bg-element p-2" rows="2" aria-label="Message" placeholder="Message #{roomName(roomId)}"
      bind:value={chat.drafts[roomId]} onkeydown={onKey} oninput={onInput} onpaste={onPaste}></textarea>
    <div class="flex items-center gap-2">
      <label class="flex-1 text-sm">
        Attach a file
        <input type="file" class="text-xs" onchange={(e) => { void attach([...(e.currentTarget.files ?? [])]); e.currentTarget.value = ''; }} />
      </label>
      <button class="rounded bg-selected px-3 py-1" disabled={sending || progress !== null}>Send</button>
    </div>
  </form>
</section>
