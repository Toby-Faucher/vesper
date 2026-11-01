<script lang="ts">
  import { motion } from '../../lib/ui/motion';
  import { SETTINGS_NAMES as NAMES, SETTINGS_SECTIONS, href, type SettingsSection } from '../../lib/route';
  import { wsUrl } from '../../lib/session.svelte';
  import { voice } from '../../lib/voice.svelte';
  import Diagnostics from '../Diagnostics.svelte';
  import Account from './Account.svelte';
  import Appearance from './Appearance.svelte';
  import Notifications from './Notifications.svelte';
  import Voice from './Voice.svelte';
  // A lazy chunk: the section list and the open section. A move inside Settings replaces the history entry (navigate), so
  // the whole visit to Settings is one entry and Close (Back) returns to the page it was opened from.
  let { section }: { section: SettingsSection } = $props();
</script>

<div class="flex min-h-0 flex-1 gap-8 overflow-y-auto px-4 py-6">
  <nav aria-label="Settings sections" class="sticky top-0 w-50 shrink-0 self-start">
    <ul class="flex flex-col gap-px">
      {#each SETTINGS_SECTIONS as s (s)}
        <li>
          <a href={href({ name: 'settings', section: s })} aria-current={s === section ? 'page' : undefined}
            class="flex h-8 items-center rounded-sm px-2 text-fg-2 hover:bg-hover hover:text-fg aria-[current=page]:bg-selected aria-[current=page]:text-fg">{NAMES[s]}</a>
        </li>
      {/each}
    </ul>
  </nav>
  {#key section}
  <div class="min-w-0 max-w-160 flex-1 pb-12" in:motion={'swap'}>
    <h2 class="mb-6 text-lg font-medium text-fg">{NAMES[section]}</h2>
    {#if section === 'voice'}
      <Voice />
    {:else if section === 'notifications'}
      <Notifications />
    {:else if section === 'diagnostics'}
      <Diagnostics pc={voice.pc} capture={voice.capture} wsUrl={wsUrl()} mode={voice.mode} reason={voice.modeReason}
        status={voice.joined ? 'joined' : 'not joined'} />
    {:else if section === 'appearance'}
      <Appearance />
    {:else}
      <Account />
    {/if}
  </div>
  {/key}
</div>
