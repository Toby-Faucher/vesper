<script lang="ts">
  import * as I from '../lib/ui/icons';
  import type { Peer } from '../lib/gen/Peer';
  import { session } from '../lib/session.svelte';
  import { voice } from '../lib/voice.svelte';
  import { togglePerson } from '../lib/ui/person.svelte';
  import { motion } from '../lib/ui/motion';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import Speaking from '../lib/ui/c/Speaking.svelte';
  import LinkIcon from '../lib/ui/c/LinkIcon.svelte';
  // The people in voice under a room row. Folded, it is a stack of avatars on one line. Speaking is known only for the call
  // you are in; other rooms send no level, so they show no ring. Each person is a button that opens their card. A person who
  // joins or leaves slides in or out (the first and the last with the whole list); folding and speaking change at once.
  let { roomId, roomName, peers, folded }: { roomId: number; roomName: string; peers: Peer[]; folded: boolean } = $props();

  const STACK = 8; // avatars drawn when folded; the rest is "+n"
  const user = (p: Peer) => session.state.users[p.user_id] ?? { user_id: p.user_id, display_name: p.name, accent: '' };
  const speaking = (p: Peer) => voice.joinedRoom === roomId && voice.speaking.has(p.user_id);
  const link = (p: Peer) => (voice.joinedRoom === roomId ? voice.links[p.user_id] : undefined); // only the call you are in has stats
  const state = (p: Peer) => (p.deafened ? 'deafened' : p.muted || p.force_muted ? 'muted' : null);
</script>

{#if peers.length}<div in:motion={'row'} out:motion={'row'}>
{#if folded}
  <ul class="flex h-7 items-center pr-2 pl-7" aria-label="In voice in {roomName}">
    {#each peers.slice(0, STACK) as p (p.user_id)}
      {@const s = speaking(p)}
      <li class="stacked mr-1" class:speaking={s} data-speaking={s ? 'true' : 'false'} in:motion={'row'} out:motion={'row'}>
        <button type="button" class="flex rounded-full" title={p.name} onclick={(e) => togglePerson(p.user_id, e.currentTarget)}>
          <Avatar user={user(p)} size={20} speaking={s} />
          <span class="sr-only">{p.name}{s ? ', speaking' : ''}{state(p) ? `, ${state(p)}` : ''}</span>
        </button>
      </li>
    {/each}
    {#if peers.length > STACK}<li class="ml-2 text-xs text-fg-2 tabular-nums">+{peers.length - STACK}</li>{/if}
  </ul>
{:else}
  <ul aria-label="In voice in {roomName}">
    {#each peers as p (p.user_id)}
      {@const s = speaking(p)}
      {@const st = state(p)}
      <li class="flex min-w-0" class:speaking={s} data-speaking={s ? 'true' : 'false'} in:motion={'row'} out:motion={'row'}>
        <button type="button" class="person flex h-7 min-w-0 flex-1 items-center gap-2 rounded-sm pr-2 pl-7 text-left text-fg-2"
          onclick={(e) => togglePerson(p.user_id, e.currentTarget)}>
          <Avatar user={user(p)} size={20} speaking={s} />
          <span class="min-w-0 truncate" title={p.name}>{p.name}</span>
          {#if s}<Speaking />{/if}
          {#if st}
            <span class="flex shrink-0 items-center text-danger" title={st === 'deafened' ? 'Deafened' : 'Muted'}>
              <Icon d={st === 'deafened' ? I.headphonesOff : I.micOff} /><span class="sr-only">{st}</span>
            </span>
          {/if}
          {#if p.share}<span class="flex shrink-0 items-center text-fg-3" role="img" aria-label="Sharing screen" title="Sharing screen"><Icon d={I.monitor} /></span>{/if}
          {#if link(p)}<span class="ml-auto flex"><LinkIcon link={link(p)!} name={p.user_id === voice.userId ? null : p.name} /></span>{/if}
        </button>
      </li>
    {/each}
  </ul>
{/if}
</div>{/if}

<style>
  /* Folded avatars sit 4 px apart, not overlapped: an overlap cut the speaking ring (4 px outside the avatar) in half. */
  .stacked { position: relative; display: flex; }
  .stacked.speaking { z-index: 1; }
  /* Folded: hover lifts the avatar over its neighbours and draws its ring in the border colour; focus lifts it too, so
     the global focus ring is not cut by the next avatar. */
  .stacked:hover, .stacked:focus-within { z-index: 2; }
  .stacked button:hover :global(.av) { box-shadow: 0 0 0 2px var(--line-strong); }
  .person { transition: background-color var(--dur-1) var(--ease-out); }
  .person:hover { background: var(--bg-hover); color: var(--fg); }
  .person:focus-visible { outline-offset: -2px; } /* the ring draws over the next avatar, not under it */
</style>
