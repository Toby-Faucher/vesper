<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../../lib/api';
  import { UI_MESSAGES as M } from '../../lib/ui/messages';
  import { bytes } from './admin';
  import Button from '../../lib/ui/c/Button.svelte';
  import Status from '../../lib/ui/c/Status.svelte';
  import type { DiskUse } from '../../lib/gen/DiskUse';
  // Admin > Disk use, the owner's. One read of GET /api/admin/disk, again on Refresh. The server reports no backup state,
  // so this page says nothing about backups.
  let d = $state<DiskUse | null>(null);
  let failed = $state(false);
  let loading = $state(false);
  async function load() {
    if (loading) return;
    loading = true;
    try {
      d = await api<DiskUse>('GET', '/api/admin/disk');
      failed = false;
    } catch {
      failed = true;
    } finally {
      loading = false;
    }
  }
  onMount(load);
  // The database has no limit of its own and is a sliver of the disk: its size only, no bar (of 0).
  const rows = $derived(
    d && [
      { label: 'Uploads', used: d.upload_bytes, of: d.quota_bytes, text: `${bytes(d.upload_bytes)} of ${bytes(d.quota_bytes)}` },
      { label: 'Database', used: d.db_bytes, of: 0, text: `${bytes(d.db_bytes)}` },
      { label: 'Disk', used: d.fs_total - d.fs_free, of: d.fs_total, text: `${bytes(d.fs_total - d.fs_free)} of ${bytes(d.fs_total)}` },
    ],
  );
  const pct = (used: number, of: number) => (of > 0 ? Math.min(100, Math.round((used / of) * 100)) : 0);
</script>

<div class="flex max-w-140 flex-col gap-6">
  {#if failed}
    <p role="alert" class="text-sm text-fg-2">{M.diskFailed}</p>
  {:else if !rows}
    <p class="text-sm text-fg-2">{M.diskLoading}</p>
  {:else}
    {#if d?.warn}<Status kind="warn">{M.diskWarn}</Status>{/if}
    <div class="flex flex-col gap-5">
      {#each rows as r (r.label)}
        {@const p = pct(r.used, r.of)}
        <div class="flex flex-col gap-2">
          <div class="flex items-baseline justify-between gap-4 text-sm">
            <span class="font-medium text-fg">{r.label}</span>
            <span class="tabular-nums text-fg-2">{r.text}{r.of ? `, ${p} %` : ''}</span>
          </div>
          {#if r.of}
            <div class="track" role="meter" aria-label={r.label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={p} aria-valuetext={r.text}>
              <div class="bar" class:hot={p >= 80} style:width="{p}%"></div>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
  <div><Button loading={loading} onclick={load}>Refresh</Button></div>
</div>

<style>
  .track { height: 8px; overflow: hidden; border-radius: 4px; background: var(--bg-selected); box-shadow: inset 0 0 0 1px var(--line-strong); }
  .bar { height: 100%; min-width: 2px; border-radius: 4px; background: var(--fg-2); }
  .hot { background: var(--warn-fg); }
</style>
