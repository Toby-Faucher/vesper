<script lang="ts">
  import { api, errorText } from '../lib/api';
  import type { ForceMute } from '../lib/gen/ForceMute';
  import type { UserInfo } from '../lib/gen/UserInfo';
  import { session } from '../lib/session.svelte';
  import { setVolume, voice } from '../lib/voice.svelte';
  import type { IconName } from '../lib/ui/icons';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import Dialog from '../lib/ui/c/Dialog.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import Popover from '../lib/ui/c/Popover.svelte';
  import Range from '../lib/ui/c/Range.svelte';
  // One person: who they are, where they are in voice, their volume when they are in your call, and the moderator voice
  // actions when you outrank them (the server refuses the rest). A popover under `anchor`, or a dialog for the switcher.
  let {
    userId,
    anchor,
    side = 'right',
    asDialog = false,
    onclose,
  }: { userId: string; anchor: HTMLElement | null; side?: 'right' | 'top'; asDialog?: boolean; onclose: () => void } = $props();

  const RANK = { member: 0, moderator: 1, owner: 2 } as const;
  const ROLE = { member: 'Member', moderator: 'Moderator', owner: 'Owner' } as const;
  const me = $derived(session.state.me?.user);
  let busy = $state('');
  let error = $state('');
  // Someone who signed up after this tab connected is not in the hello's user list (no event announces them): ask once.
  let fetched = $state<UserInfo>();
  const user = $derived(session.state.users[userId] ?? fetched);
  $effect(() => {
    if (!session.state.users[userId]) api<UserInfo[]>('GET', '/api/users').then((l) => (fetched = l.find((u) => u.user_id === userId)), (e) => (error = errorText(e)));
  });
  const self = $derived(userId === me?.user_id);
  // The server does not echo your own join, so your place comes from the call.
  const peer = $derived(session.state.voice.find((p) => p.user_id === userId));
  const name = $derived(user?.display_name ?? peer?.name ?? 'Someone');
  const roomId = $derived(self ? voice.joinedRoom : peer?.room_id ?? null);
  const where = $derived(roomId === null ? 'Not in voice' : `In voice in ${session.state.rooms.find((r) => r.id === roomId)?.name ?? 'a room'}`);
  const inCall = $derived(!self && voice.joined && roomId === voice.joinedRoom);
  const volume = $derived(voice.volumes[userId] ?? 100);
  const outranks = $derived(!!me && !!user && !self && RANK[me.role] > RANK[user.role] && me.role !== 'member');

  let confirming = $state(false);
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

{#snippet action(icon: IconName, text: string, what: 'force-mute' | 'kick' | 'disconnect', run: () => void, danger = false)}
  <!-- Busy ignores clicks but stays focusable (not disabled), so focus is not dropped while a request runs. -->
  <button type="button" class="act" class:danger aria-busy={busy === what || undefined} aria-disabled={!!busy || undefined} onclick={() => busy || run()}>
    <Icon name={icon} /><span class="truncate">{text}</span>
  </button>
{/snippet}

{#snippet body()}
  <div class="flex w-68 flex-col gap-3 p-2">
    <div class="flex items-center gap-3">
      {#if user}<Avatar {user} size={40} muted={peer?.muted || peer?.force_muted} deafened={peer?.deafened} />{/if}
      <div class="min-w-0">
        <p class="truncate text-base font-medium text-fg" title={name}>{name}</p>
        <p class="truncate text-xs text-fg-2">{#if user}@{user.username} · {ROLE[user.role]}{/if}</p>
      </div>
    </div>
    <p class="flex items-center gap-2 text-fg-2"><Icon name={roomId === null ? 'headphones-off' : 'headphones'} />{where}</p>

    {#if inCall}
      <div class="flex flex-col gap-2 border-t border-subtle pt-3">
        <div class="flex items-center justify-between">
          <span class="text-fg-2">Volume</span>
          <span class="flex items-center gap-2">
            <span class="text-fg tabular-nums">{volume} %</span>
            {#if volume !== 100}<button type="button" class="link" onclick={() => setVolume(userId, 100)}>Reset</button>{/if}
          </span>
        </div>
        <Range min={0} max={200} value={volume} aria-label="Volume for {name}" aria-valuetext="{volume} %"
          oninput={(e) => setVolume(userId, +e.currentTarget.value)} />
        <!-- Always laid out, so the card does not grow under the slider (it is placed once, on open). -->
        <p class="text-xs text-fg-3" class:invisible={volume <= 100}>Above 100 % can distort.</p>
      </div>
    {/if}

    {#if outranks}
      <div class="flex flex-col border-t border-subtle pt-2">
        {#if peer}
          {@render action(peer.force_muted ? 'mic' : 'mic-off', peer.force_muted ? 'Allow to speak' : 'Mute for everyone', 'force-mute',
            () => act('force-mute', { muted: !peer.force_muted }))}
          {@render action('phone-off', 'Remove from voice', 'kick', () => act('kick'))}
        {/if}
        {@render action('log-out', 'Disconnect', 'disconnect', () => (confirming = true), true)}
      </div>
      <Dialog open={confirming} title="Disconnect {name}?" onclose={() => (confirming = false)}>
        Every open tab of theirs loses its connection to the server. They can connect again.
        {#snippet footer()}
          <Button variant="ghost" onclick={() => (confirming = false)}>Cancel</Button>
          <Button variant="danger" loading={busy === 'disconnect'} onclick={() => act('disconnect')}>Disconnect</Button>
        {/snippet}
      </Dialog>
    {/if}
    {#if error}<p role="alert" class="text-danger">{error}</p>{/if}
  </div>
{/snippet}

{#if asDialog}
  <Dialog open title={name} {onclose}>{@render body()}</Dialog>
{:else}
  <Popover {anchor} open label={name} {onclose} {side}>{@render body()}</Popover>
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
  .link { color: var(--fg-2); text-decoration: underline; text-underline-offset: 2px; }
  .link:hover { color: var(--fg); }
</style>
