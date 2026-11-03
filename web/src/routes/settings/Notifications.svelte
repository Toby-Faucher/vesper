<script lang="ts">
  import Button from '../../lib/ui/c/Button.svelte';
  import Field from '../../lib/ui/c/Field.svelte';
  import Range from '../../lib/ui/c/Range.svelte';
  import Switch from '../../lib/ui/c/Switch.svelte';
  import PushSettings from './PushSettings.svelte';
  import { chime } from '../../lib/ui/chime';
  import type { Cue } from '../../lib/ui/cues';
  import { loadPrefs, permission, requestMentions, savePrefs, testNotification, type NotifyPrefs } from '../../lib/ui/notify';
  import { voice } from '../../lib/voice.svelte';
  // Settings > Notifications: mention notifications (the browser asks only when the switch is turned on) and the call
  // sounds. Mentions and the join and leave of others are off until turned on, my own call cues are on. Saved to vesper.notify.
  let prefs = $state(loadPrefs());
  let perm = $state(permission());
  const on = $derived(prefs.mentions && perm === 'granted');
  const dmsOn = $derived(prefs.dms && perm === 'granted');
  const STATE: Record<typeof perm, string> = {
    granted: 'Allowed',
    denied: 'Blocked by your browser. Allow notifications for this site in the address bar, then turn this on again.',
    default: 'Not asked yet',
    unsupported: 'This browser does not show notifications.',
  };
  type Group = 'muteCues' | 'connectCues' | 'sounds';
  const GROUPS: { key: Group; label: string; help: string; cues: Cue[] }[] = [
    { key: 'muteCues', label: 'Mute and deafen', help: 'When you mute, unmute, deafen or undeafen.', cues: ['mute', 'unmute', 'deafen', 'undeafen'] },
    { key: 'connectCues', label: 'Connection', help: 'When you join or leave a call, and when the connection is lost.', cues: ['connect', 'disconnect', 'lost'] },
    { key: 'sounds', label: 'Others joining and leaving', help: 'When someone joins or leaves your call.', cues: ['join', 'leave'] },
  ];

  async function setKind(key: 'mentions' | 'dms', e: Event & { currentTarget: HTMLInputElement }) {
    const box = e.currentTarget;
    if (box.checked) perm = await requestMentions([key]); // turns it on when allowed
    else savePrefs({ ...loadPrefs(), [key]: false });
    prefs = loadPrefs();
    box.checked = key === 'mentions' ? on : dmsOn; // a refusal leaves the switch off
  }
  function set(p: Partial<NotifyPrefs>) {
    savePrefs({ ...loadPrefs(), ...p });
    prefs = loadPrefs();
  }
  let sample: ReturnType<typeof setTimeout>[] = [];
  function play(cues: Cue[]) {
    sample.forEach(clearTimeout); // a second click restarts, never overlaps
    const one = (c: Cue) => chime(c, voice.settings.speakerId, prefs.volume);
    one(cues[0]); // inside the click, so the browser lets the sound start
    sample = cues.slice(1).map((c, i) => setTimeout(() => one(c), (i + 1) * 450));
  }
</script>

<!-- Permission can change in the address bar while this page is open. -->
<svelte:window onfocus={() => (perm = permission())} />

<div class="sections">
  <section role="group" aria-labelledby="notify-mentions">
    <div class="flex h-8 items-center"><h3 id="notify-mentions" class="text-base font-medium text-fg">Mentions and direct messages</h3></div>
    <div>
      <Switch label="Notify me when someone mentions me" checked={on} disabled={perm === 'unsupported'}
        aria-describedby="mention-state" onchange={(e) => setKind('mentions', e)} />
      <p id="mention-state" role="status" class="-mt-1 pr-11 text-xs text-fg-2">{STATE[perm]}</p>
      <Switch label="Notify me when someone sends me a direct message" checked={dmsOn} disabled={perm === 'unsupported'}
        onchange={(e) => setKind('dms', e)} />
      {#if perm === 'granted'}
        <div class="mt-2"><Button size="sm" onclick={testNotification}>Send a test notification</Button></div>
      {/if}
    </div>
  </section>

  <PushSettings />

  <section role="group" aria-labelledby="notify-sounds">
    <div class="flex h-8 items-center"><h3 id="notify-sounds" class="text-base font-medium text-fg">Sounds</h3></div>
    <Switch label="Call sounds" help="Short, quiet tones on your speaker. They are not sent into the call, but on open speakers your mic can pick them up. Echo cancellation removes most of that. With headphones nobody else hears them."
      checked={prefs.call} onchange={(e) => set({ call: e.currentTarget.checked })} />
    <ul class="groups flex flex-col gap-3">
      {#each GROUPS as g (g.key)}
        <li>
          <Switch label={g.label} help={g.help} checked={prefs[g.key]} disabled={!prefs.call}
            onchange={(e) => set({ [g.key]: e.currentTarget.checked })} />
          <span class="-ml-2"><Button variant="ghost" size="sm" aria-label="Play a sample: {g.label}" disabled={!prefs.call || !prefs.volume}
            onclick={() => play(g.cues)}>Play a sample</Button></span>
        </li>
      {/each}
    </ul>
    <Field label="Volume" value={String(prefs.volume)}>
      <Range min={0} max={100} value={prefs.volume} disabled={!prefs.call} aria-valuetext="{prefs.volume} percent"
        oninput={(e) => set({ volume: +e.currentTarget.value })} />
    </Field>
  </section>
</div>

<style>
  .sections { display: flex; flex-direction: column; gap: 32px; }
  section { display: flex; flex-direction: column; gap: 16px; }
  .sections > :global(section + section) { padding-top: 24px; border-top: 1px solid var(--line-subtle); }
  .groups { padding-left: 12px; border-left: 1px solid var(--line-subtle); }
</style>
