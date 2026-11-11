<script lang="ts">
  import * as I from '../lib/ui/icons';
  import type { Peer } from '../lib/gen/Peer';
  import { inRoom } from '../lib/peers';
  import { href } from '../lib/route';
  import { sortDms } from '../lib/state';
  import { navigate, session } from '../lib/session.svelte';
  import { voice } from '../lib/voice.svelte';
  import { setCollapsed } from '../lib/ui/sidebar.svelte';
  import { glide } from '../lib/ui/motion';
  import { layers } from '../lib/ui/layers.svelte';
  import { isMac } from '../lib/ui/keys';
  import { shares } from '../lib/share/state.svelte';
  import { UI_MESSAGES } from '../lib/ui/messages-first';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';
  import Mark from '../lib/ui/c/Mark.svelte';
  import Menu from '../lib/ui/c/Menu.svelte';
  import MenuItem from '../lib/ui/c/MenuItem.svelte';
  import RoomRow from './RoomRow.svelte';
  // The one sidebar: the mark, the rooms in the owner's order with who is in voice, and you with the account menu (Admin,
  // Log out) and the Settings gear.
  let { current }: { current: number | null } = $props();

  const rooms = $derived([...session.state.rooms].sort((a, b) => a.position - b.position));
  const dms = $derived(sortDms(session.state.dms, session.state.unread, session.state.activity));
  const me = $derived(session.state.me?.user);
  const staff = $derived(me?.role === 'owner' || me?.role === 'moderator');
  // The server tells everyone else about your join, not you: your own row comes from the call itself.
  const self = $derived<Peer | null>(
    me && voice.joined && voice.joinedRoom !== null
      ? { user_id: me.user_id, name: me.display_name, room_id: voice.joinedRoom, muted: voice.muted, deafened: voice.deafened, force_muted: voice.forceMuted, share: shares.mine && { fps: shares.mine.fps, bitrate_kbps: shares.mine.kbps } }
      : null,
  );
  const peersIn = (id: number) => {
    const list = inRoom(session.state.voice, id).filter((p) => p.user_id !== self?.user_id);
    return self?.room_id === id ? [self, ...list] : list;
  };
  let accountEl = $state<HTMLButtonElement | null>(null);
  // Another room opened: its selected fill glides over from the one before.
  let nav: HTMLElement;
  let was: HTMLElement | null = null;
  $effect.pre(() => {
    void current;
    was = nav?.querySelector('.row.current') ?? null;
  });
  $effect(() => {
    void current;
    const now = nav.querySelector<HTMLElement>('.row.current');
    if (was && now) glide(was, now);
  });
  let menuOpen = $state(false);
</script>

<aside class="flex h-screen w-62 flex-col border-r border-subtle bg-sidebar">
  <div class="flex h-12 shrink-0 items-center gap-2 pr-2 pl-4">
    <Mark size={28} />
    <span class="ml-auto flex">
      <IconButton icon={I.command} label="Switch to ({isMac() ? '⌘' : 'Ctrl'} K)" tip aria-haspopup="dialog" aria-keyshortcuts={isMac() ? 'Meta+K' : 'Control+K'}
        onclick={() => (layers.switcher = true)} />
      <IconButton icon={I.sidebar} label="Hide sidebar" tip aria-expanded="true" aria-controls="sidebar" onclick={() => setCollapsed(true)} />
    </span>
  </div>

  <nav bind:this={nav} aria-label="Rooms" class="relative isolate min-h-0 flex-1 overflow-y-auto px-2 pb-3">
    {#if dms.length}
      <h2 class="flex h-7 items-center px-2 text-xs text-fg-3">Direct messages</h2>
      <ul class="mb-2 flex flex-col gap-px">
        {#each dms as room (room.id)}
          <RoomRow {room} current={room.id === current} peers={peersIn(room.id)} />
        {/each}
      </ul>
    {/if}
    <h2 class="flex h-7 items-center px-2 text-xs text-fg-3">Rooms</h2>
    {#if rooms.length}
      <ul class="flex flex-col gap-px">
        {#each rooms as room (room.id)}
          <RoomRow {room} current={room.id === current} peers={peersIn(room.id)} />
        {/each}
      </ul>
    {:else if session.online}
      <p class="px-2 text-sm text-fg-3">{UI_MESSAGES.noRooms}</p>
    {/if}
  </nav>

  {#if me}
    <div class="flex h-12 shrink-0 items-center gap-1 border-t border-subtle px-2">
      <button bind:this={accountEl} type="button" aria-label="Account, {me.display_name}" onclick={() => (menuOpen = !menuOpen)}
        class="account flex h-8 min-w-0 flex-1 items-center gap-2 rounded-sm px-2 text-left">
        <Avatar user={me} size={20} />
        <span class="min-w-0 truncate text-fg" title={me.display_name}>{me.display_name}</span>
        <span class="ml-auto flex shrink-0 text-fg-2"><Icon d={I.chevronDown} /></span>
      </button>
      <span class="cog flex"><IconButton icon={I.settings} label="Settings" tip onclick={() => navigate(href({ name: 'settings', section: 'voice' }))} /></span>
      <Menu anchor={accountEl} open={menuOpen} onclose={() => (menuOpen = false)} label="Account">
        {#if staff}<MenuItem icon={I.shield} onselect={() => navigate(href({ name: 'admin', section: 'invites' }))}>Admin</MenuItem>{/if}
        <MenuItem icon={I.logOut} onselect={() => session.logout()}>Log out</MenuItem>
      </Menu>
    </div>
  {/if}
</aside>

<style>
  .account { transition: background-color var(--dur-1) var(--ease-out); }
  .account:hover { background: var(--bg-hover); }
  /* The gear turns a third of a turn (its six teeth line up again) on hover or keyboard focus. None under reduced motion. */
  @media (prefers-reduced-motion: no-preference) {
    .cog :global(svg) { transition: transform var(--dur-3) var(--ease-out); }
    .cog :global(button:hover svg),
    .cog :global(button:focus-visible svg) { transform: rotate(120deg); }
  }
</style>
