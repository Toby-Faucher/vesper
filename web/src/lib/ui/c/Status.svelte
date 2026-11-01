<script lang="ts">
  import * as I from '../icons';
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  // A status line: icon plus text. Info is plain text; warn, error and ok sit on their subtle surface.
  // Error is role="alert" (announced at once), the rest role="status".
  let { kind, children }: { kind: 'info' | 'warn' | 'error' | 'ok'; children: Snippet } = $props();
  const icon = { info: I.info, warn: I.warn, error: I.error, ok: I.check };
</script>

<div role={kind === 'error' ? 'alert' : 'status'} class="status {kind}">
  <span class="mark"><Icon d={icon[kind]} /></span>
  <div class="min-w-0">{@render children()}</div>
</div>

<style>
  .status {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    color: var(--fg);
    font-size: 13px;
    line-height: 20px;
  }
  .mark { display: flex; height: 20px; align-items: center; }
  .warn, .error, .ok { padding: 6px 10px; border-radius: var(--radius); }
  .info { color: var(--fg-2); }
  .info .mark { color: var(--fg-3); }
  .warn { background: var(--warn-subtle); }
  .warn .mark { color: var(--warn-fg); }
  .error { background: var(--danger-subtle); }
  .error .mark { color: var(--danger-fg); }
  .ok { background: var(--ok-subtle); }
  .ok .mark { color: var(--ok-fg); }
</style>
