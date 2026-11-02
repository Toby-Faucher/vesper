<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { relTime } from '../../lib/ui/messages';
  import { callLine } from './account';
  import type { CallRecord } from '../../lib/gen/CallRecord';
  // Settings > Account: your own last calls, as your client reported them at leave. Nobody else sees this list.
  let calls = $state<CallRecord[] | null>(null);
  let failed = $state(false);
  let now = $state(Date.now());
  onMount(() => {
    api<CallRecord[]>('GET', '/api/me/calls').then((l) => ([calls, now] = [l, Date.now()]), () => (failed = true));
  });
</script>

<section role="group" aria-labelledby="account-calls">
  <div>
    <h3 id="account-calls" class="text-base font-medium text-fg">Your recent calls</h3>
    <p class="mt-1 text-sm text-fg-2">How your last calls went, as your browser measured them. Only you can see this list, not the owner or anyone else. Vesper keeps your last 20 calls for 30 days.</p>
  </div>
  {#if failed}
    <p role="alert" class="text-sm text-fg-2">Your recent calls could not be loaded. Reload the page to try again.</p>
  {:else if !calls}
    <p class="text-sm text-fg-2">Loading your recent calls.</p>
  {:else if !calls.length}
    <p class="text-sm text-fg-2">No calls yet. A call shows here after you leave it.</p>
  {:else}
    <ul aria-label="Your recent calls" class="calls">
      {#each calls as c (c.id)}
        {@const l = callLine(c)}
        <li class="px-3 py-2">
          <p class="text-sm text-fg"><span title={new Date(c.ended_at).toLocaleString()}>{relTime(c.ended_at, now)}</span>: {l.head}</p>
          <p class="text-xs text-fg-2 tabular-nums">{l.figures}</p>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  section { display: flex; flex-direction: column; gap: 16px; }
  .calls { border: 1px solid var(--line); border-radius: var(--radius); }
  .calls > li + li { border-top: 1px solid var(--line-subtle); }
</style>
