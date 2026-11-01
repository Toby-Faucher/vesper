<script lang="ts">
  import { SHORTCUTS, isMac, keysFor } from '../lib/ui/keys';
  import Kbd from '../lib/ui/c/Kbd.svelte';
  // Every shortcut, from the same table the keys are read with, plus the switcher's filters. In the shortcuts dialog and
  // in Settings > Voice.
  const mac = isMac();
  const FILTERS = [['r', 'Only rooms'], ['p', 'Only people'], ['>', 'Only actions']];
  // The message list: a row and its actions (MessageItem), and Esc back to the composer (Timeline).
  const MESSAGES: [string, string[]][] = [
    ['Previous or next message', ['↑', '↓']],
    ['Move along the message actions', ['←', '→']],
    ['Newest message and the message box', ['Esc']],
  ];
</script>

{#snippet row(label: string, keys: string[])}
  <div class="flex h-8 items-center gap-4">
    <dt class="min-w-0 flex-1 text-fg-2">{label}</dt>
    <dd class="flex shrink-0 gap-1">{#each keys as k, i (i)}<Kbd>{k}</Kbd>{/each}</dd>
  </div>
{/snippet}

<div>
<dl class="flex flex-col">
  {#each SHORTCUTS as d, i (i)}{@render row(d.label, keysFor(d, mac))}{/each}
</dl>
<h4 class="mt-4 flex h-7 items-center text-xs font-medium text-fg-3">In the switcher, start with</h4>
<dl class="flex flex-col">
  {#each FILTERS as [k, label] (k)}{@render row(label, [k, 'Space'])}{/each}
</dl>
<h4 class="mt-4 flex h-7 items-center text-xs font-medium text-fg-3">In the messages</h4>
<dl class="flex flex-col">
  {#each MESSAGES as [label, keys] (label)}{@render row(label, keys)}{/each}
</dl>
<p class="mt-4 text-xs text-fg-3">Mute and deafen keys work only in a call. Outside a call those keys stay the browser's.</p>
</div>
