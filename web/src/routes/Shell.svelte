<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { href, parse } from '../lib/route';
  import { navigate, route, session } from '../lib/session.svelte';
  import { startVoice, voice } from '../lib/voice.svelte';
  import { closePerson, person } from '../lib/ui/person.svelte';
  import { setCollapsed, sidebar } from '../lib/ui/sidebar.svelte';
  import { UI_MESSAGES } from '../lib/ui/messages';
  import Button from '../lib/ui/c/Button.svelte';
  import EmptyState from '../lib/ui/c/EmptyState.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';
  import CallBar from './CallBar.svelte';
  import ConnectionBanner from './ConnectionBanner.svelte';
  import RoomView from './RoomView.svelte';
  import Sidebar from './Sidebar.svelte';
  // The signed-in app: the sidebar and the routed page. It lives as long as the sign-in, so the call does too: startVoice()
  // runs once here, and its cleanup (sign out) ends the call and resets its state.
  onMount(startVoice);

  const LAST = 'vesper.lastRoom';
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
  $effect(() => {
    if (room) try { storage?.setItem(LAST, String(room.id)); } catch { /* storage blocked */ }
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
    navigate(a.pathname);
  }
  let barH = $state(0); // the pane keeps this much room below its content for the call bar
  const title = $derived(r.name === 'settings' ? 'Settings' : r.name === 'admin' ? 'Admin' : '');
</script>

<!-- The handler only catches clicks on links, and Enter on a link is a click. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="grid h-screen overflow-hidden {sidebar.collapsed ? 'grid-cols-[1fr]' : 'grid-cols-[248px_1fr]'}" {onclick}>
  {#if !sidebar.collapsed}<Sidebar current={room?.id ?? null} />{/if}
  <main class="relative flex h-screen min-w-0 flex-col" style:padding-bottom={voice.joined ? `${barH + 24}px` : null}>
    <ConnectionBanner />
    {#if room}
      <RoomView {room} />
    {:else if r.name === 'room' || r.name === 'home' || r.name === 'welcome'}
      {#if loaded}
        {@render pageHeader('')}
        {#if r.name === 'room'}
          <EmptyState title={UI_MESSAGES.roomMissing} body={UI_MESSAGES.roomMissingBody}>
            {#snippet action()}{#if rooms[0]}<Button onclick={() => navigate(href({ name: 'room', id: rooms[0].id }))}>{UI_MESSAGES.openFirstRoom(rooms[0].name)}</Button>{/if}{/snippet}
          </EmptyState>
        {:else}
          <EmptyState title={UI_MESSAGES.noRooms} body={UI_MESSAGES.noRoomsBody} />
        {/if}
      {/if}
    {:else if r.name === 'settings'}
      {@render pageHeader(title, true)}
      {#await import('./settings/SettingsPage.svelte') then m}<m.default section={r.section} />{/await}
    {:else if r.name === 'admin'}
      {@render pageHeader(title, true)}
      {#await import('./admin/AdminPage.svelte') then m}<m.default section={r.section} />{/await}
    {:else}
      {@render pageHeader('')}
      <EmptyState title={UI_MESSAGES.pageMissing}>
        {#snippet action()}<Button onclick={() => navigate('/')}>{UI_MESSAGES.goHome}</Button>{/snippet}
      </EmptyState>
    {/if}
    {#if voice.joined}<CallBar bind:height={barH} />{/if}
  </main>
</div>

{#if person.id}
  {#await import('./PersonCard.svelte') then m}
    {#key person.anchor}<m.default userId={person.id} anchor={person.anchor} side={person.side} onclose={closePerson} />{/key}
  {/await}
{/if}

{#snippet pageHeader(text: string, closable = false)}
  {#if text || sidebar.collapsed}
  <header class="flex h-12 shrink-0 items-center gap-3 border-b border-subtle px-4">
    {#if sidebar.collapsed}
      <span class="-ml-2"><IconButton icon="sidebar" label="Show sidebar" tip onclick={() => setCollapsed(false)} /></span>
    {/if}
    {#if text}<h1 class="min-w-0 truncate text-base font-medium text-fg">{text}</h1>{/if}
    {#if closable}<span class="ml-auto"><Button variant="ghost" onclick={() => navigate('/')}>Close</Button></span>{/if}
  </header>
  {/if}
{/snippet}
