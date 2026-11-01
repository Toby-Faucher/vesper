<script lang="ts">
  import type { Peer } from '../lib/gen/Peer';
  import { session } from '../lib/session.svelte';
  import { voice } from '../lib/voice.svelte';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import Speaking from '../lib/ui/c/Speaking.svelte';
  // The people in voice under a room row. Folded, it is a stack of avatars on one line. Speaking is known only for the call
  // you are in; other rooms send no level, so they show no ring.
  let { roomId, roomName, peers, folded }: { roomId: number; roomName: string; peers: Peer[]; folded: boolean } = $props();

  const STACK = 8; // avatars drawn when folded; the rest is "+n"
  const user = (p: Peer) => session.state.users[p.user_id] ?? { user_id: p.user_id, display_name: p.name, accent: '' };
  const speaking = (p: Peer) => voice.joinedRoom === roomId && voice.speaking.has(p.user_id);
  const state = (p: Peer) => (p.deafened ? 'deafened' : p.muted || p.force_muted ? 'muted' : null);
</script>

{#if folded}
  <ul class="flex h-7 items-center pr-2 pl-7" aria-label="In voice in {roomName}">
    {#each peers.slice(0, STACK) as p (p.user_id)}
      {@const s = speaking(p)}
      <li class="stacked -mr-1" class:speaking={s} data-speaking={s ? 'true' : 'false'} title={p.name}>
        <Avatar user={user(p)} size={20} speaking={s} />
        <span class="sr-only">{p.name}{s ? ', speaking' : ''}{state(p) ? `, ${state(p)}` : ''}</span>
      </li>
    {/each}
    {#if peers.length > STACK}<li class="ml-2 text-xs text-fg-2 tabular-nums">+{peers.length - STACK}</li>{/if}
  </ul>
{:else}
  <ul aria-label="In voice in {roomName}">
    {#each peers as p (p.user_id)}
      {@const s = speaking(p)}
      {@const st = state(p)}
      <li class="flex h-7 min-w-0 items-center gap-2 pr-2 pl-7 text-fg-2" class:speaking={s} data-speaking={s ? 'true' : 'false'}>
        <Avatar user={user(p)} size={20} speaking={s} />
        <span class="min-w-0 truncate" title={p.name}>{p.name}</span>
        {#if s}<Speaking />{/if}
        {#if st}
          <span class="flex shrink-0 items-center text-danger" title={st === 'deafened' ? 'Deafened' : 'Muted'}>
            <Icon name={st === 'deafened' ? 'headphones-off' : 'mic-off'} /><span class="sr-only">{st}</span>
          </span>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  /* The stack's overlap: each avatar sits on a 2 px ring of the sidebar so the edges read. */
  .stacked { position: relative; display: flex; }
  .stacked :global(.av) { box-shadow: 0 0 0 2px var(--bg-sidebar); }
  .stacked.speaking { z-index: 1; } /* the ring draws over the next avatar, not under it */
</style>
