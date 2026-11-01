<script lang="ts">
  import type { Snippet } from 'svelte';
  import { profileLabel, togglePerson } from '../person.svelte';
  // The one way a name opens someone's card: a plain button named "Open profile of <name>" that looks like the text it
  // wraps (no box, so nothing moves). Esc on the card gives focus back here (Popover).
  let { id, name, class: cls = '', el = $bindable(), children }: { id: string; name: string; class?: string; el?: HTMLButtonElement; children: Snippet } = $props();
</script>

<button type="button" bind:this={el} class="who {cls}" aria-label={profileLabel(name)} onclick={(e) => togglePerson(id, e.currentTarget)}>{@render children()}</button>

<style>
  .who { display: inline; padding: 0; border-radius: var(--radius-sm); font: inherit; color: inherit; text-align: inherit; }
  .who:hover { text-decoration: underline; text-underline-offset: 2px; }
</style>
