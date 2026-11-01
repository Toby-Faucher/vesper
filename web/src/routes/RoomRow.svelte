<script lang="ts">
  import * as I from '../lib/ui/icons';
  import type { Peer } from '../lib/gen/Peer';
  import type { RoomInfo } from '../lib/gen/RoomInfo';
  import { href } from '../lib/route';
  import { session } from '../lib/session.svelte';
  import Badge from '../lib/ui/c/Badge.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import PresenceList from './PresenceList.svelte';
  // One room in the sidebar: a link with its unread and mention pills, and the people in voice there below it.
  let { room, current, peers }: { room: RoomInfo; current: boolean; peers: Peer[] } = $props();

  const FOLD_ABOVE = 5; // more people than this fold into an avatar stack until the chevron opens them
  let fold = $state<boolean | null>(null); // the reader's choice; null follows the count
  const folded = $derived(fold ?? peers.length > FOLD_ABOVE);
  const u = $derived(session.state.unread[room.id]);
  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;
  const label = $derived(
    [room.name, room.restricted && 'restricted', u?.unread && `${u.unread} unread`, u?.mentions && plural(u.mentions, 'mention'),
      peers.length && `${peers.length} in voice`].filter(Boolean).join(', '),
  );
</script>

<li>
  <div class="row group flex h-7 items-center rounded-sm" class:current class:unread={!!u?.unread}>
    <a href={href({ name: 'room', id: room.id })} aria-current={current ? 'page' : undefined} aria-label={label} title={room.name}
      class="flex h-full min-w-0 flex-1 items-center gap-1.5 rounded-sm px-2">
      <span class="name min-w-0 truncate">{room.name}</span>
      {#if room.restricted}<span class="meta flex shrink-0"><Icon d={I.shield} /></span>{/if}
      <span class="ml-auto flex shrink-0 items-center gap-1 pl-1">
        {#if u?.unread}<Badge kind="count" n={u.unread} pop />{/if}
        {#if u?.mentions}<Badge kind="mention" n={u.mentions} pop />{/if}
      </span>
    </a>
    {#if peers.length}
      <button type="button" class="meta mr-1 flex size-5 shrink-0 items-center justify-center rounded-sm"
        aria-expanded={!folded} aria-label="{folded ? 'Show' : 'Hide'} people in voice in {room.name}" onclick={() => (fold = !folded)}>
        <Icon d={folded ? I.chevronRight : I.chevronDown} />
      </button>
    {/if}
  </div>
  <PresenceList roomId={room.id} roomName={room.name} {peers} {folded} />
</li>

<style>
  .row { color: var(--fg-2); transition: background-color var(--dur-1) var(--ease-out); }
  .row:hover { background: var(--bg-hover); }
  .row.unread { color: var(--fg); font-weight: var(--weight-strong); }
  .row.current { background: var(--bg-selected); color: var(--fg); }
  /* tertiary on the sidebar and on hover; never on the selected surface (4.26:1), where it steps up to --fg-2 */
  .meta { color: var(--fg-3); }
  .current .meta { color: var(--fg-2); }
  button.meta:hover { color: var(--fg); }
  .row a:focus-visible, .row button:focus-visible { outline-offset: -2px; }
</style>
