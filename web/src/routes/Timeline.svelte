<script lang="ts">
  // The scrolling list of one room. Every message `article` is a direct child of the scroll container (the e2e measures
  // an article against its parentElement); day and New rows are siblings. It follows the bottom only when already
  // there, keeps the view still when older pages load above, opens at the New row when the first unread message is
  // loaded, and jumps to a message (chat.jump) with a short flash.
  import { onMount, tick, untrack } from 'svelte';
  import MessageItem from './MessageItem.svelte';
  import { errorText } from '../lib/api';
  import { anchorTop, chat, jumpTo, loadLatest, loadNewer, loadOlder, markRead, openRoom, trimRooms } from '../lib/chat.svelte';
  import type { Message } from '../lib/gen/Message';
  import { parse } from '../lib/markdown';
  import { session } from '../lib/session.svelte';
  import * as I from '../lib/ui/icons';
  import { fadeOut, flash } from '../lib/ui/motion';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import { firstUnreadInPage, plain, rows } from '../lib/ui/timeline';
  import Button from '../lib/ui/c/Button.svelte';
  import EmptyState from '../lib/ui/c/EmptyState.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';

  let {
    roomId,
    act,
    onreply,
    onempty,
  }: {
    roomId: number;
    act: (f: () => Promise<unknown>) => Promise<boolean>;
    onreply: (m: Message) => void;
    onempty: () => void;
  } = $props();

  let list = $state<HTMLElement>();
  let newEl = $state<HTMLElement>();
  let stuck = $state(true); // scrolled to the bottom: follow new messages
  let ready = false; // the room's first scroll is done
  let loading = $state(true);
  let loadError = $state('');
  let loadingOlder = $state(false);
  let readAt = $state(0); // last_read_id when the room opened: the New row stays put while it is read
  let unreadAt = 0;
  let firstPill = $state(0); // "N new messages. Show the first" when the first unread is older than the page
  let toNew = false; // after "Show the first": scroll to the New row once it is loaded
  let faded = $state(false);
  let announce = $state(''); // the newest message from someone else, for screen readers
  let lastId: number | null = null; // newest id seen; null right after a room switch, so history is not announced
  let seenLatest = chat.latest;
  let now = $state(Date.now());
  const readSent: Record<number, number> = {};
  const FADE_AFTER_MS = 4000;

  const log = $derived(chat.state.rooms[roomId]);
  const myId = $derived(session.state.me?.user.user_id ?? '');
  const myName = $derived(session.state.me?.user.username ?? '');
  const isMod = $derived(session.state.me?.user.role === 'owner' || session.state.me?.user.role === 'moderator');
  const user = (id: string) => session.state.users[id] ?? { user_id: id, display_name: 'Someone', accent: '' };
  // The New row only where the first unread message really is: not at the top of a page that starts after it.
  const newFrom = $derived(log && firstUnreadInPage(log.messages, readAt, myId, log.moreBefore) !== null ? readAt : Infinity);
  const items = $derived(rows(log?.messages ?? [], newFrom, myId, now));
  const empty = $derived(!loading && !loadError && log && !log.messages.length && !log.moreBefore);

  onMount(() => {
    const t = setInterval(() => (now = Date.now()), 60_000); // "Today" becomes "Yesterday" at midnight
    return () => {
      clearInterval(t);
      clearTimeout(fadeTimer);
      chat.active = null;
    };
  });

  // A new room: load it (or the slice a jump asked for), then place the view once.
  $effect(() => {
    const room = roomId;
    untrack(() => open(room));
  });

  function open(room: number) {
    ready = false;
    stuck = true;
    loading = true;
    loadError = '';
    firstPill = 0;
    toNew = false;
    faded = false;
    clearTimeout(fadeTimer);
    lastId = null;
    const u = session.state.unread[room];
    [readAt, unreadAt] = [u?.last_read_id ?? 0, u?.unread ?? 0];
    // A copied message link, /r/<room>#m<id>: load around it and flash it.
    const link = /^#m(\d{1,15})$/.exec(location.hash);
    if (link) {
      chat.jump = Number(link[1]);
      history.replaceState(history.state, '', location.pathname);
    }
    const p = link ? jumpTo(room, chat.jump!).then(() => openRoom(room, true)) : openRoom(room, chat.jump !== null);
    p.then(
      async () => {
        if (room !== roomId) return;
        loading = false;
        await tick();
        place();
      },
      (e) => {
        if (room !== roomId) return;
        loading = false;
        loadError = errorText(e);
        chat.jump = null;
      },
    );
  }

  // The first view of a room: a jump target, else the New row when it is loaded, else the bottom (with the pill when
  // the unread messages start before the page).
  function place() {
    if (!list || !log) return;
    ready = true;
    if (chat.jump === null) {
      if (newFrom !== Infinity && newEl && !log.moreAfter) {
        reveal(newEl, 'start');
        stuck = atBottom(list);
      } else {
        if (unreadAt > 0 && log.messages.length && newFrom === Infinity) firstPill = unreadAt;
        toBottom();
      }
    }
    if (empty) onempty();
    afterChange();
  }

  // After the list changed: scroll to a jump target, or follow the bottom, then announce, trim and mark read.
  $effect(() => {
    void log?.messages;
    untrack(afterChange);
  });

  function afterChange() {
    if (!ready || !list || !log) return;
    if (chat.jump !== null) {
      const el = document.getElementById(`m${chat.jump}`);
      if (el) {
        reveal(el, 'center');
        flash(el);
        chat.jump = null;
        stuck = atBottom(list);
      }
    } else if (toNew && newEl) {
      toNew = false;
      reveal(newEl, 'start');
      stuck = atBottom(list);
    } else if (stuck) toBottom();
    const msgs = log.messages;
    const last = msgs.at(-1);
    if (last && lastId !== null && last.id > lastId) {
      const since = lastId;
      const fresh = msgs.filter((m) => m.id > since);
      // A live message or two, not a page loaded with "Load newer".
      const m = fresh.length <= 3 ? fresh.reverse().find((x) => x.author_id !== myId && !x.deleted) : undefined;
      if (m) announce = `${user(m.author_id).display_name}: ${plain(parse(m.body))}`;
    }
    lastId = last?.id ?? 0;
    trimRooms(roomId, stuck && !loadingOlder);
    maybeRead();
  }

  // The list fills the pane, so it shrinks when the call bar or the connection banner appear: keep the bottom in view.
  $effect(() => {
    if (!list) return;
    const ro = new ResizeObserver(() => stuck && toBottom());
    ro.observe(list);
    return () => ro.disconnect();
  });

  // Esc (the shell raises chat.latest): go to the newest message.
  $effect(() => {
    const n = chat.latest;
    untrack(() => {
      if (n === seenLatest) return;
      seenLatest = n;
      void toLatest();
    });
  });

  // The New row fades a few seconds after the room is marked read (a room that fits on screen is read the moment it
  // opens, and the line should still be seen). It keeps its height, so nothing below moves.
  let fadeTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const read = session.state.unread[roomId]?.last_read_id ?? 0;
    const last = log?.messages.at(-1)?.id ?? 0;
    untrack(() => {
      if (!newEl || faded || !last || read < last || read <= readAt) return;
      faded = true;
      const el = newEl;
      fadeTimer = setTimeout(() => fadeOut(el), FADE_AFTER_MS);
    });
  });

  // Scrolls only the list: scrollIntoView would also scroll the clipped pane and the page around it.
  function reveal(el: HTMLElement, at: 'start' | 'center') {
    if (!list) return;
    list.scrollTop = at === 'start' ? el.offsetTop - 48 : el.offsetTop - (list.clientHeight - el.offsetHeight) / 2;
  }

  function toBottom() {
    if (list) list.scrollTop = list.scrollHeight;
  }
  const atBottom = (el: HTMLElement) => el.scrollHeight - el.scrollTop - el.clientHeight < 8;

  async function toLatest() {
    chat.jump = null;
    toNew = false;
    stuck = true;
    if (log?.moreAfter) await act(() => loadLatest(roomId));
    toBottom();
  }

  // Mark read only what the reader can see: the newest page, scrolled to the bottom, in a visible tab.
  function maybeRead() {
    const last = log?.messages.at(-1);
    if (!last || log?.moreAfter || !stuck || document.visibilityState !== 'visible') return;
    if ((session.state.unread[roomId]?.last_read_id ?? 0) >= last.id || (readSent[roomId] ?? 0) >= last.id) return;
    const room = roomId;
    readSent[room] = last.id;
    markRead(room, last.id).catch(() => (readSent[room] = 0));
  }

  function markAll() {
    const last = log?.messages.at(-1);
    if (last) void act(() => markRead(roomId, last.id));
  }

  async function older() {
    if (loadingOlder || !list) return;
    loadingOlder = true;
    const [top, height] = [list.scrollTop, list.scrollHeight];
    if (await act(() => loadOlder(roomId))) {
      await tick();
      list.scrollTop = anchorTop(top, height, list.scrollHeight);
    }
    loadingOlder = false;
  }

  function onScroll() {
    if (!list) return;
    stuck = atBottom(list);
    if (list.scrollTop < 40 && log?.moreBefore && ready) void older();
    maybeRead();
  }

  /** A reply quote: scroll to the original and flash it, loading the history around it when it is not here. */
  function jump(id: number) {
    const el = document.getElementById(`m${id}`);
    if (el) {
      reveal(el, 'center');
      flash(el);
      stuck = atBottom(list!);
      return;
    }
    chat.jump = id;
    void act(() => jumpTo(roomId, id)).then((ok) => ok || (chat.jump = null));
  }

  function showFirst() {
    firstPill = 0;
    toNew = true;
    void act(() => jumpTo(roomId, readAt)).then((ok) => ok || (toNew = false));
  }
</script>

<svelte:document onvisibilitychange={maybeRead} />

<div class="relative flex min-h-0 flex-1 flex-col">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex: the scroll region takes focus so the keyboard can scroll it -->
  <div bind:this={list} onscroll={onScroll} data-timeline class="relative min-h-0 flex-1 overflow-y-auto pt-2 pb-4" tabindex="0" role="region" aria-label="Messages">
    {#if loadError}
      <div class="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
        <p class="text-sm text-danger" role="alert">{loadError}</p>
        <Button onclick={() => open(roomId)}>Try again</Button>
      </div>
    {:else if loading && !log?.messages.length}
      <p class="px-4 pt-2 text-sm text-fg-3" role="status">{UI_MESSAGES.chatLoading}</p>
    {:else if empty}
      <div class="flex h-full items-center justify-center"><EmptyState title="No messages yet." body="Say hello." /></div>
    {/if}
    {#if log?.moreBefore}
      <div class="py-2 pl-15"><Button size="sm" variant="ghost" loading={loadingOlder} onclick={older}>Load older messages</Button></div>
    {/if}
    {#each items as r (r.key)}
      {#if r.kind === 'day'}
        <div class="day" role="separator" aria-label={r.label}><span aria-hidden="true">{r.label}</span></div>
      {:else if r.kind === 'new'}
        <!-- Not one role=separator: its children would be hidden from screen readers, and the button must stay. -->
        <div class="new" bind:this={newEl} inert={faded}>
          <span class="line" role="separator" aria-label="New messages"></span>
          <span class="label" aria-hidden="true">New</span>
          <button type="button" class="mark" onclick={markAll}>Mark as read</button>
        </div>
      {:else}
        <MessageItem m={r.m} head={r.head} {myId} {myName} {isMod} {user} {act} {onreply} onjump={jump} />
      {/if}
    {/each}
    {#if log?.moreAfter}
      <div class="py-2 pl-15"><Button size="sm" variant="ghost" onclick={() => act(() => loadNewer(roomId))}>Load newer messages</Button></div>
    {/if}
  </div>

  {#if firstPill}
    <button type="button" class="pill top" onclick={showFirst}>{firstPill} new {firstPill === 1 ? 'message' : 'messages'}. Show the first</button>
  {/if}
  {#if !stuck || log?.moreAfter}
    <button type="button" class="pill bottom" aria-label="Jump to latest" onclick={toLatest}><Icon d={I.arrowDown} />Jump to latest</button>
  {/if}
  <p class="sr-only" aria-live="polite">{announce}</p>
</div>

<style>
  .day {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 16px 16px 0 68px;
    color: var(--fg-3);
    font-size: 12px;
    font-weight: 500;
    line-height: 16px;
  }
  .day::after { content: ''; flex: 1; height: 1px; background: var(--line-subtle); }
  .new {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 24px;
    margin: 8px 16px 0 68px;
  }
  .new .line { flex: 1; height: 1px; background: var(--accent-line); }
  .new .label { color: var(--accent-fg); font-size: 12px; font-weight: 500; line-height: 16px; }
  .mark {
    padding: 0 8px;
    border-radius: var(--radius-sm);
    color: var(--fg-2);
    font-size: 12px;
    line-height: 24px;
  }
  .mark:hover { background: var(--bg-hover); color: var(--fg); }
  .pill {
    position: absolute;
    display: inline-flex;
    height: 32px;
    align-items: center;
    gap: 6px;
    padding: 0 12px;
    border: 1px solid var(--line);
    border-radius: 999px;
    background: var(--bg-raised);
    color: var(--fg);
    font-size: 13px;
    line-height: 20px;
  }
  .pill:hover { border-color: var(--line-strong); background: var(--bg-hover); }
  .pill.top { top: 8px; left: 50%; translate: -50% 0; border-color: var(--accent-line); color: var(--accent-fg); }
  .pill.bottom { right: 24px; bottom: 12px; }
</style>
