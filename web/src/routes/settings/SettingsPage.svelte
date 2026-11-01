<script lang="ts">
  import { SETTINGS_SECTIONS, href, type SettingsSection } from '../../lib/route';
  import { wsUrl } from '../../lib/session.svelte';
  import { updateSettings, voice } from '../../lib/voice.svelte';
  import { UI_MESSAGES } from '../../lib/ui/messages';
  import Settings from '../Settings.svelte';
  import Diagnostics from '../Diagnostics.svelte';
  // A lazy chunk. Task 12 groups the voice controls into sections; until then Voice holds today's settings card.
  let { section }: { section: SettingsSection } = $props();
  const NAMES: Record<SettingsSection, string> = {
    voice: 'Voice', notifications: 'Notifications', appearance: 'Appearance', account: 'Account', diagnostics: 'Diagnostics',
  };
</script>

<div class="flex min-h-0 flex-1 gap-8 overflow-y-auto px-4 py-6">
  <nav aria-label="Settings sections" class="w-50 shrink-0">
    <ul class="flex flex-col gap-px">
      {#each SETTINGS_SECTIONS as s (s)}
        <li>
          <a href={href({ name: 'settings', section: s })} aria-current={s === section ? 'page' : undefined}
            class="flex h-7 items-center rounded-sm px-2 text-fg-2 hover:bg-hover aria-[current=page]:bg-selected aria-[current=page]:text-fg">{NAMES[s]}</a>
        </li>
      {/each}
    </ul>
  </nav>
  <div class="min-w-0 max-w-160 flex-1">
    <h2 class="mb-4 text-lg font-medium text-fg">{NAMES[section]}</h2>
    {#if section === 'voice'}
      <Settings settings={voice.settings} joinedBitrate={voice.joinedBitrate} joined={voice.joined} onchange={updateSettings}
        liveStream={voice.mic} sinkSupported={'setSinkId' in AudioContext.prototype} />
    {:else if section === 'diagnostics'}
      <Diagnostics pc={voice.pc} capture={voice.capture} wsUrl={wsUrl()} mode={voice.mode} status={voice.joined ? 'joined' : 'not joined'} />
    {:else}
      <p class="text-fg-3">{UI_MESSAGES.sectionLater}</p>
    {/if}
  </div>
</div>
