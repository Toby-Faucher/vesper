<script lang="ts">
  import * as I from '../lib/ui/icons';
  import type { ForceMute } from '../lib/gen/ForceMute';
  import { api, errorText } from '../lib/api';
  import { navigate, session } from '../lib/session.svelte';
  import { href } from '../lib/route';
  import type { RoomInfo } from '../lib/gen/RoomInfo';
  import { findRoom } from '../lib/state';
  import { canModerate } from '../lib/ui/person.svelte';
  import { loadUsers } from '../lib/ui/users';
  import { setVolume, voice } from '../lib/voice.svelte';
  import { MAX_VOLUME } from '../lib/volume';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Dialog from '../lib/ui/c/Dialog.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import LinkIcon from '../lib/ui/c/LinkIcon.svelte';
  import { MEASURING, linkDetail } from '../lib/ui/link';
  import Popover from '../lib/ui/c/Popover.svelte';
  import Range from '../lib/ui/c/Range.svelte';
  // One person: who they are, where they are in voice, their volume when they are in your call, and the moderator voice
  // actions when you outrank them (the server refuses the rest). A popover under `anchor`, or a dialog for the switcher.
  let {
    userId,
    anchor,
    side = 'right',
    above = null,
    asDialog = false,
    onclose,
  }: { userId: string; anchor: HTMLElement | null; side?: 'right' | 'top'; above?: Element | null; asDialog?: boolean; onclose: () => void } = $props();

  const ROLE = { member: 'Member', moderator: 'Moderator', owner: 'Owner' } as const;
  const me = $derived(session.state.me?.user);
  let busy = $state('');
  let error = $state('');
  let missing = $state(false); // the list could not be loaded, or does not have them: say so, and show no actions
  const user = $derived(session.state.users[userId]);
  $effect(() => {
    if (session.state.users[userId]) return;
    loadUsers().then(() => (missing = !session.state.users[userId]), () => (missing = true));
  });
  const self = $derived(userId === me?.user_id);
  // The server does not echo your own join, so your place comes from the call.
  const peer = $derived(session.state.voice.find((p) => p.user_id === userId));
  const name = $derived(user?.display_name ?? peer?.name ?? 'Someone');
  const roomId = $derived(self ? voice.joinedRoom : peer?.room_id ?? null);
  const where = $derived(roomId === null ? 'Not in voice' : `In voice in ${findRoom(session.state, roomId)?.name ?? 'a room'}`);
  const inCall = $derived(!self && voice.joined && roomId === voice.joinedRoom);
  // In a call the row is always there (measuring until the first stats poll): the card is placed once, on open, at its height.
  const link = $derived(voice.joined && (self || inCall) ? voice.links[userId] ?? MEASURING : undefined);
  const volume = $derived(voice.volumes[userId] ?? 100);
  const outranks = $derived(canModerate(me, user));

  let confirming = $state(false);
  // Opens the DM (made on the first time) and goes to it.
  async function message() {
    busy = 'dm';
    error = '';
    try {
      const room = await api<RoomInfo>('POST', '/api/dms', { user_id: userId });
      onclose();
      navigate(href({ name: 'room', id: room.id }));
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = '';
    }
  }
  async function act(what: 'force-mute' | 'kick' | 'disconnect', body?: ForceMute) {
    busy = what;
    error = '';
    try {
      await api('POST', `/api/users/${userId}/${what}`, body);
      if (what !== 'force-mute') onclose();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = '';
      confirming = false;
    }
  }
</script>

{#snippet action(icon: I.IconPath, text: string, what: 'force-mute' | 'kick' | 'disconnect', run: () => void, danger = false)}
  <!-- Busy ignores clicks but stays focusable (not disabled), so focus is not dropped while a request runs. -->
  <button type="button" class="act" class:danger aria-busy={busy === what || undefined} aria-disabled={!!busy || undefined} onclick={() => busy || run()}>
    <Icon d={icon} /><span class="truncate">{text}</span>
  </button>
{/snippet}

{#snippet body()}
  <div class="flex w-68 flex-col gap-3 p-2">
    <div class="flex items-center gap-3">
      {#if user}<Avatar {user} size={40} muted={peer?.muted || peer?.force_muted} deafened={peer?.deafened} />{/if}
      <div class="min-w-0">
        <p class="truncate text-base font-medium text-fg" title={name}>{name}</p>
        <p class="truncate text-xs text-fg-2">{#if user}@{user.username} · {ROLE[user.role]}{user.banned ? ' · Banned' : ''}{/if}</p>
      </div>
    </div>
    {#if user?.bio}<p class="bio text-fg">{user.bio}</p>{/if}
    <p class="flex items-center gap-2 text-fg-2"><Icon d={roomId === null ? I.headphonesOff : I.headphones} />{where}</p>
    {#if link}
      <div class="flex items-start gap-2 text-fg-2">
        <span class="flex h-5 items-center"><LinkIcon {link} name={self ? null : name} /></span>
        <p>{#if link.grade !== 'measuring'}<span class="text-fg">Connection {link.grade}.</span>{' '}{/if}<span class="tabular-nums">{linkDetail(link)}</span></p>
      </div>
    {/if}

    {#if !self && user && !user.banned}
      <button type="button" class="act" aria-busy={busy === 'dm' || undefined} aria-disabled={!!busy || undefined} onclick={() => busy || message()}>
        <Icon d={I.send} /><span class="truncate">Message</span>
      </button>
    {/if}

    {#if inCall}
      <div class="flex flex-col gap-2 border-t border-subtle pt-3">
        <!-- The note sits in the label row, so the card keeps its height (it is placed once, on open) and leaves no gap. -->
        <div class="flex items-center gap-2">
          <span class="mr-auto text-fg-2">Volume</span>
          {#if volume > 100}<span class="text-xs text-fg-3">Can distort</span>{/if}
          <span class="text-fg tabular-nums">{volume} %</span>
          {#if volume !== 100}<button type="button" class="link" onclick={() => setVolume(userId, 100)}>Reset</button>{/if}
        </div>
        <Range min={0} max={MAX_VOLUME} value={volume} aria-label="Volume for {name}" aria-valuetext="{volume} %{volume > 100 ? ', above 100 % can distort' : ''}"
          oninput={(e) => setVolume(userId, +e.currentTarget.value)} />
      </div>
    {/if}

    {#if outranks}
      <div class="flex flex-col border-t border-subtle pt-2">
        {#if peer}
          {@render action(peer.force_muted ? I.mic : I.micOff, peer.force_muted ? 'Allow to speak' : 'Mute for everyone', 'force-mute',
            () => act('force-mute', { muted: !peer.force_muted }))}
          {@render action(I.phoneOff, 'Remove from voice', 'kick', () => act('kick'))}
        {/if}
        {@render action(I.logOut, 'Disconnect', 'disconnect', () => (confirming = true), true)}
      </div>
      <Dialog open={confirming} title="Disconnect {name}?" onclose={() => (confirming = false)}>
        Every open tab of theirs loses its connection to the server. They can connect again.
        {#snippet footer()}
          <Button variant="ghost" onclick={() => (confirming = false)}>Cancel</Button>
          <Button variant="danger" loading={busy === 'disconnect'} onclick={() => act('disconnect')}>Disconnect</Button>
        {/snippet}
      </Dialog>
    {/if}
    {#if missing}<p role="status" class="text-xs text-fg-3">Could not load this person's details.</p>{/if}
    {#if error}<p role="alert" class="text-danger">{error}</p>{/if}
  </div>
{/snippet}

{#if asDialog}
  <Dialog open title={name} {onclose}>{@render body()}</Dialog>
{:else}
  <Popover {anchor} open label={name} {onclose} {side} {above}>{@render body()}</Popover>
{/if}

<style>
  .act {
    display: flex;
    height: 32px;
    align-items: center;
    gap: 8px;
    margin: 0 -8px;
    padding: 0 8px;
    border-radius: var(--radius-sm);
    color: var(--fg);
    text-align: left;
    transition: background-color var(--dur-1) var(--ease-out);
  }
  .act :global(svg) { color: var(--fg-2); }
  .act:hover:not([aria-disabled]) { background: var(--bg-hover); }
  .act.danger, .act.danger :global(svg) { color: var(--danger-fg); }
  .act[aria-busy] { cursor: progress; }
  .act:focus-visible { outline-offset: -2px; }
  .bio { overflow-wrap: anywhere; }
  .link { color: var(--fg-2); text-decoration: underline; text-underline-offset: 2px; }
  .link:hover { color: var(--fg); }
</style>
