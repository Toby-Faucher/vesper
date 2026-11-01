<script lang="ts">
  import { getContext, tick, type Snippet } from 'svelte';
  import type { IconName } from '../icons';
  import Icon from './Icon.svelte';
  let {
    onselect,
    icon,
    danger = false,
    disabled = false,
    children,
  }: { onselect: () => void; icon?: IconName; danger?: boolean; disabled?: boolean; children: Snippet } = $props();
  const menu = getContext<{ close: () => void }>('vesper.menu');

  // Close first and let focus settle on the anchor, so a dialog opened by onselect returns focus there.
  async function activate() {
    menu.close();
    await tick();
    onselect();
  }
</script>

<button type="button" role="menuitem" tabindex="-1" {disabled} class:danger onclick={activate}>
  {#if icon}<Icon name={icon} />{/if}
  <span class="truncate">{@render children()}</span>
</button>

<style>
  button {
    display: flex;
    width: 100%;
    height: 32px;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    border-radius: var(--radius-sm);
    color: var(--fg);
    text-align: left;
  }
  button :global(svg) { color: var(--fg-2); }
  button:hover:enabled,
  button:focus { background: var(--bg-hover); }
  button:focus-visible { outline-offset: -2px; }
  .danger,
  .danger :global(svg) { color: var(--danger-fg); }
  button:disabled { cursor: not-allowed; color: var(--fg-disabled); }
</style>
