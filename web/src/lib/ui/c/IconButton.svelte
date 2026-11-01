<script lang="ts">
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import type { IconPath } from '../icons';
  import Icon from './Icon.svelte';
  // `pressed` given means a toggle (aria-pressed); `tip` shows the label as the CSS tooltip from app.css, below the button,
  // or above it with 'up' (near the bottom edge). `danger` draws the icon in the danger colour (a state that is on, like muted).
  let {
    icon,
    label,
    pressed,
    tip = false,
    danger = false,
    el = $bindable(null),
    ...rest
  }: {
    icon: IconPath;
    label: string;
    pressed?: boolean;
    tip?: boolean | 'up';
    danger?: boolean;
    el?: HTMLButtonElement | null; // bind:el to anchor a Popover or Menu
  } & Omit<HTMLButtonAttributes, 'children'> = $props();
</script>

<button bind:this={el} type="button" {...rest} class="ib" class:danger aria-label={label} aria-pressed={pressed} data-tip={tip ? label : undefined} data-tip-up={tip === 'up' || undefined}>
  <Icon d={icon} />
</button>

<style>
  .ib {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: var(--radius);
    color: var(--fg-2);
    transition: background-color var(--dur-1) var(--ease-out), color var(--dur-1) var(--ease-out), transform var(--dur-1) var(--ease-out);
  }
  @media (prefers-reduced-motion: no-preference) { .ib:active:enabled { transform: scale(0.95); } } /* press feedback */
  .ib:hover:enabled { background: var(--bg-hover); color: var(--fg); }
  .ib[aria-pressed='true'] { background: var(--bg-selected); color: var(--fg); }
  .ib.danger:enabled { color: var(--danger-fg); }
  .ib:disabled { cursor: not-allowed; color: var(--fg-disabled); }
</style>
