<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { href } from '../../lib/route';
  import { bytes, uptimeText } from './admin';
  import Button from '../../lib/ui/c/Button.svelte';
  import type { ServerStatus } from '../../lib/gen/ServerStatus';
  // Admin > Server, the owner's: one read of GET /api/admin/server, again on Refresh. The server does not track when its
  // public address last changed, so that is not shown.
  let s = $state<ServerStatus | null>(null);
  let failed = $state(false);
  let loading = $state(false);
  async function load() {
    if (loading) return;
    loading = true;
    try {
      s = await api<ServerStatus>('GET', '/api/admin/server');
      failed = false;
    } catch {
      failed = true;
    } finally {
      loading = false;
    }
  }
  onMount(load);
  const b = $derived(s?.last_backup);
  const rows = $derived(
    s && [
      ['Version', `${s.server.version}, build ${s.server.build}`],
      ['Running for', uptimeText(s.uptime_s)],
      ['Database', bytes(s.disk.db_bytes)],
      ['Disk free', `${bytes(s.disk.fs_free)} of ${bytes(s.disk.fs_total)}`],
      ['In voice now', String(s.voice)],
      ['Failed pushes, last 24 h', String(s.push_failures)],
      ['Last backup', b ? `${new Date(b.at).toLocaleString()}, ${b.name}` : 'None in the backups folder'],
    ],
  );
</script>

<div class="flex max-w-140 flex-col gap-6">
  {#if failed}
    <p role="alert" class="text-sm text-fg-2">Could not load the server status. Check your connection and try again.</p>
  {:else if !rows}
    <p class="text-sm text-fg-2">Loading the server status...</p>
  {:else}
    <dl class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-3 text-sm">
      {#each rows as [k, v] (k)}
        <dt class="text-fg-2">{k}</dt>
        <dd class="min-w-0 break-words tabular-nums text-fg">{v}</dd>
      {/each}
    </dl>
    <p class="text-sm text-fg-2">Uploads and the quota: <a class="text-fg underline" href={href({ name: 'admin', section: 'disk' })}>Disk use</a>.</p>
  {/if}
  <div><Button loading={loading} onclick={load}>Refresh</Button></div>
</div>
