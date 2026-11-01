<script lang="ts">
  import * as I from '../lib/ui/icons';
  import { onMount, untrack } from 'svelte';
  import { href, parse } from '../lib/route';
  import { lastRoomKey, navigate, route, session } from '../lib/session.svelte';
  import { startVoice, toggleDeafen, toggleMute, voice } from '../lib/voice.svelte';
  import { closePerson, person } from '../lib/ui/person.svelte';
  import { setCollapsed, sidebar } from '../lib/ui/sidebar.svelte';
  import { layers } from '../lib/ui/layers.svelte';
  import { chat } from '../lib/chat.svelte';
  import { isMac, isTyping, nextUnread, shortcut, stepRoom } from '../lib/ui/keys';
  import { loadRecent, pushRecent } from '../lib/ui/recent';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import Button from '../lib/ui/c/Button.svelte';
  import EmptyState from '../lib/ui/c/EmptyState.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';
  import Dialog from '../lib/ui/c/Dialog.svelte';
  import { lazy } from '../lib/ui/lazy';
  import SwitcherField from './SwitcherField.svelte';
  import { MAX_QUERY } from '../lib/ui/searchq';
  import CallBar from './CallBar.svelte';
  import ConnectionBanner from './ConnectionBanner.svelte';
  import RoomView from './RoomView.svelte';
  import Sidebar from './Sidebar.svelte';
  // The signed-in app: the sidebar and the routed page. It lives as long as the sign-in, so the call does too: startVoice()
  // runs once here, and its cleanup (sign out) ends the call and resets its state.
  onMount(startVoice);

  const LAST = $derived(lastRoomKey(session.state.me?.user.user_id ?? ''));
  const storage = (() => { try { return localStorage; } catch { return null; } })();
  const r = $derived(parse(route.path));
  const rooms = $derived([...session.state.rooms].sort((a, b) => a.position - b.position));
  const loaded = $derived(session.online || rooms.length > 0); // the room list came with hello
  const room = $derived(r.name === 'room' ? rooms.find((x) => x.id === r.id) : undefined);

  // `/` (and /welcome until Task 11) opens the last room, else the first.
  $effect(() => {
    if ((r.name !== 'home' && r.name !== 'welcome') || !loaded) return;
    const last = Number(storage?.getItem(LAST));
    const to = rooms.find((x) => x.id === last) ?? rooms[0];
    if (to) untrack(() => navigate(href({ name: 'room', id: to.id }), true));
  });
  const uid = $derived(session.state.me?.user.user_id ?? '');
  let recent = $state<number[]>([]);
  $effect(() => {
    if (room) try { storage?.setItem(LAST, String(room.id)); } catch { /* storage blocked */ }
    recent = room ? pushRecent(storage, uid, room.id) : loadRecent(storage, uid);
  });
  // The open room left the list (deleted, or made private): go to the first room, without a history entry.
  let had: number | null = null;
  $effect(() => {
    const id = room?.id ?? null;
    if (had !== null && id === null && r.name === 'room' && r.id === had) untrack(() => navigate('/', true));
    had = id;
  });

  // Same-origin links open in the app without a page load. Downloads, new tabs and /api go to the browser.
  function onclick(e: MouseEvent) {
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    const a = (e.target as Element).closest?.('a');
    if (!a || a.target || a.hasAttribute('download') || a.origin !== location.origin || a.pathname.startsWith('/api/')) return;
    e.preventDefault();
    navigate(a.pathname + a.search + a.hash);
  }
  // The switcher and the shortcuts list are one chunk, loaded on first use. The switcher's dialog is here, in the first
  // load, so it opens at once with a field that takes the typing (not the composer) and a Loading line. A failed load
  // says so and is not kept: Ctrl+K, or closing and opening again, tries again.
  const loadLayers = lazy(() => import('./switcher-chunk'));
  let layersMod = $state<Awaited<ReturnType<typeof loadLayers>>>();
  let layersFailed = $state(false);
  let typed = $state(''); // the switcher's text, also while its chunk loads
  function load() {
    layersFailed = false;
    loadLayers().then((m) => (layersMod = m), () => { layersFailed = true; layers.shortcuts = false; });
  }
  $effect(() => {
    if ((layers.switcher || layers.shortcuts) && !layersMod) untrack(load);
  });
  $effect(() => {
    if (!layers.switcher && !layersMod) typed = '';
  });
  // Message search: the same pattern, its own chunk.
  const loadSearch = lazy(() => import('./SearchDialog.svelte'));
  let searchMod = $state<Awaited<ReturnType<typeof loadSearch>>>();
  let searchFailed = $state(false);
  let searchTyped = $state('');
  function openSearch() {
    searchFailed = false;
    layers.search = true;
    if (!searchMod) loadSearch().then((m) => (searchMod = m), () => (searchFailed = true));
  }
  $effect(() => {
    if (layers.search && !searchMod && !searchFailed) untrack(openSearch);
  });
  $effect(() => {
    if (!layers.search && !searchMod) [searchTyped, searchFailed] = ['', false]; // the next open tries the chunk again
  });

  const mac = isMac();
  function onkeydown(e: KeyboardEvent) {
    if (e.defaultPrevented || e.isComposing) return;
    // Settings stops the keys itself while it waits for a new push-to-talk key, so they never reach this handler.
    const ptt = { code: voice.settings.pushToTalk.enabled ? voice.settings.pushToTalk.key : null, held: voice.pttHeld };
    const s = shortcut(e, { typing: isTyping(e.target as Element | null), mac, capturingPtt: false, ptt });
    if (!s) return;
    if (s === 'escape') {
      // Dialogs, menus and popovers close themselves on Esc; with none open, the timeline goes to the newest message.
      if (room && !document.querySelector('dialog[open], :popover-open')) chat.latest++;
      return;
    }
    if (e.repeat && s !== 'prev-room' && s !== 'next-room') return e.preventDefault(); // a held key toggles once
    const ids = rooms.map((x) => x.id);
    const to = (id: number | null) => id !== null && navigate(href({ name: 'room', id }));
    if (s === 'search') openSearch();
    else if (s === 'switcher') {
      if (layers.switcher && layersFailed) load();
      else layers.switcher = !layers.switcher;
    } else if (s === 'help') layers.shortcuts = true;
    else if (s === 'mute' || s === 'deafen') {
      if (!voice.joined) return; // not in a call: Ctrl+Shift+M and D stay the browser's
      if (s === 'mute') toggleMute();
      else toggleDeafen();
    }
    else if (s === 'sidebar') setCollapsed(!sidebar.collapsed);
    else if (s === 'prev-room' || s === 'next-room') to(stepRoom(ids, room?.id ?? null, s === 'next-room' ? 1 : -1));
    else if (s === 'next-unread') to(nextUnread(ids, room?.id ?? null, session.state.unread));
    e.preventDefault();
  }
  let mainEl = $state<HTMLElement>();
  let barH = $state(0); // the pane keeps this much room below its content for the call bar
  const title = $derived(r.name === 'settings' ? 'Settings' : r.name === 'admin' ? 'Admin' : '');
</script>

<!-- The handler only catches clicks on links, and Enter on a link is a click. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="grid h-screen overflow-hidden {sidebar.collapsed ? 'grid-cols-[1fr]' : 'grid-cols-[248px_1fr]'}" {onclick}>
  <!-- First Tab stop. It focuses main itself, so the address does not gain a #main. -->
  <a href="#main" class="skip" onclick={(e) => { e.preventDefault(); mainEl?.focus(); }}>Skip to main content</a>
  <!-- Hidden, not removed, while collapsed, so the toggles' aria-controls always points at it. -->
  <div id="sidebar" class="min-w-0" hidden={sidebar.collapsed}><Sidebar current={room?.id ?? null} /></div>
  <main id="main" tabindex="-1" bind:this={mainEl} class="relative flex h-screen min-w-0 flex-col" style:padding-bottom={voice.joined ? `${barH + 24}px` : null}>
    <ConnectionBanner />
    {#if room}
      <RoomView {room} />
    {:else if r.name === 'room' || r.name === 'home' || r.name === 'welcome'}
      {#if loaded}
        {@render pageHeader('')}
        <div class="center">
        {#if r.name === 'room'}
          <EmptyState title={UI_MESSAGES.roomMissing} body={UI_MESSAGES.roomMissingBody}>
            {#snippet action()}{#if rooms[0]}<Button onclick={() => navigate(href({ name: 'room', id: rooms[0].id }))}>{UI_MESSAGES.openFirstRoom(rooms[0].name)}</Button>{/if}{/snippet}
          </EmptyState>
        {:else}
          <EmptyState title={UI_MESSAGES.noRooms} body={UI_MESSAGES.noRoomsBody} />
        {/if}
        </div>
      {/if}
    {:else if r.name === 'settings'}
      {@render pageHeader(title, true)}
      {#await import('./settings/SettingsPage.svelte') then m}<m.default section={r.section} />{/await}
    {:else if r.name === 'admin'}
      {@render pageHeader(title, true)}
      {#await import('./admin/AdminPage.svelte') then m}<m.default section={r.section} />{/await}
    {:else}
      {@render pageHeader('')}
      <div class="center">
        <EmptyState title={UI_MESSAGES.pageMissing}>
          {#snippet action()}<Button onclick={() => navigate('/')}>{UI_MESSAGES.goHome}</Button>{/snippet}
        </EmptyState>
      </div>
    {/if}
    {#if voice.joined}<CallBar bind:height={barH} />{/if}
  </main>
</div>

<svelte:window {onkeydown} />

{#if person.id}
  {#await import('./PersonCard.svelte') then m}
    {#key person.anchor}
      <m.default userId={person.id} anchor={person.anchor} side={person.side} asDialog={!person.anchor} onclose={closePerson} />
    {/key}
  {/await}
{/if}
<Dialog open={layers.switcher} title="Switch to" bare onclose={() => (layers.switcher = false)}>
  {#if layersMod}
    <layersMod.Switcher open={layers.switcher} current={room?.id ?? null} {recent} bind:q={typed} onclose={() => (layers.switcher = false)} />
  {:else}
    <SwitcherField bind:value={typed} />
    {#if layersFailed}
      <p role="alert" class="px-4 py-6 text-center text-fg-2">{UI_MESSAGES.switcherFailed}</p>
    {:else}
      <p class="px-4 py-6 text-center text-fg-3">{UI_MESSAGES.switcherLoading}</p>
    {/if}
  {/if}
</Dialog>
<Dialog id="search" open={layers.search} title="Search" bare onclose={() => (layers.search = false)}>
  {#if searchMod}
    <searchMod.default open={layers.search} current={room?.id ?? null} bind:q={searchTyped} onclose={() => (layers.search = false)} />
  {:else if layers.search}
    <SwitcherField bind:value={searchTyped} maxlength={MAX_QUERY} aria-label="Search messages" placeholder="Search messages. Try from:alice in:general" />
    {#if searchFailed}
      <p role="alert" class="px-4 py-6 text-center text-fg-2">{UI_MESSAGES.searchFailed}</p>
    {:else}
      <p class="px-4 py-6 text-center text-fg-3">{UI_MESSAGES.switcherLoading}</p>
    {/if}
  {/if}
</Dialog>
{#if layersMod}<layersMod.Shortcuts open={layers.shortcuts} onclose={() => (layers.shortcuts = false)} />{/if}

<style>
  /* Off screen until focused, then a raised chip at the top left, over the sidebar. */
  .skip {
    position: fixed;
    top: 8px;
    left: 8px;
    z-index: 60;
    padding: 6px 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-raised);
    color: var(--fg);
    font-size: 14px;
    transform: translateY(-200%);
  }
  .skip:focus { transform: none; }
  /* Empty states sit in the middle of the pane; the bottom padding offsets the 48 px header, so they centre on the screen. */
  .center { display: flex; flex: 1; flex-direction: column; justify-content: center; padding-bottom: 48px; }
  /* Search: wider and taller than the switcher, for snippets and the filter row. */
  :global(dialog#search) { width: min(720px, calc(100vw - 32px)); max-height: min(640px, calc(100vh - 128px)); }
  main:focus-visible { outline: none; } /* the skip link's target: the whole pane, a ring there helps nobody */
</style>

{#snippet pageHeader(text: string, closable = false)}
  {#if text || sidebar.collapsed}
  <header class="flex h-12 shrink-0 items-center gap-3 border-b border-subtle px-4">
    {#if sidebar.collapsed}
      <span class="-ml-2"><IconButton icon={I.sidebar} label="Show sidebar" tip aria-expanded="false" aria-controls="sidebar" onclick={() => setCollapsed(false)} /></span>
    {/if}
    {#if text}<h1 class="min-w-0 truncate text-base font-medium text-fg">{text}</h1>{/if}
    {#if closable}<span class="ml-auto"><Button variant="ghost" onclick={() => navigate('/', true)}>Close</Button></span>{/if}
  </header>
  {/if}
{/snippet}
