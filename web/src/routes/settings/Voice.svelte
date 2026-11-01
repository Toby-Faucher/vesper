<script lang="ts">
  import { clampSettings, type AudioSettings } from '../../lib/audio/settings';
  import { NAMES, RESETS, type Section } from './sections';
  import { keyName, pttKeyProblem } from '../../lib/ui/keys';
  import { updateSettings, voice } from '../../lib/voice.svelte';
  import Button from '../../lib/ui/c/Button.svelte';
  import Dialog from '../../lib/ui/c/Dialog.svelte';
  import Field from '../../lib/ui/c/Field.svelte';
  import Kbd from '../../lib/ui/c/Kbd.svelte';
  import Range from '../../lib/ui/c/Range.svelte';
  import Select from '../../lib/ui/c/Select.svelte';
  import Switch from '../../lib/ui/c/Switch.svelte';
  import MicCheck from './MicCheck.svelte';
  import ShortcutList from '../ShortcutList.svelte';
  // Settings > Voice: every audio setting, in sections. Each change goes through updateSettings (saved to vesper.audio and
  // applied to a running call at once, as before).
  const settings = $derived(voice.settings);
  const set = (patch: Partial<AudioSettings>) => updateSettings(clampSettings({ ...settings, ...patch }));
  const sinkSupported = 'setSinkId' in AudioContext.prototype;

  let bitrateDraft = $state(64); // applied on release, not on every drag step
  $effect(() => void (bitrateDraft = settings.bitrateKbps));
  let devices = $state<MediaDeviceInfo[]>([]); // filled by MicCheck
  const speakers = $derived(devices.filter((d) => d.kind === 'audiooutput'));

  const MODES: Record<AudioSettings['mode'], string> = {
    deepfilter: 'Strongest. Runs on your computer.',
    browser: "Your browser's own filter. Lighter.",
    off: 'No filtering.',
  };

  // Push to talk key: the button listens while it has focus. Its handlers stop the keys, so neither push to talk nor the
  // shortcuts see them (the shell's capturingPtt stays false). Esc or leaving the button (Tab) cancels. Focus stays on the
  // button after a capture: the window's push-to-talk handler still gets the key and stops it from clicking the button.
  let capturing = $state(false);
  let refused = $state('');
  function captureKey(e: KeyboardEvent) {
    if (e.key === 'Tab') return; // Tab leaves the button, and the blur cancels
    e.preventDefault();
    if (e.repeat) return;
    if (e.key === 'Escape') return void ([capturing, refused] = [false, '']);
    refused = pttKeyProblem(e) ?? '';
    if (refused) return;
    capturing = false;
    set({ pushToTalk: { ...settings.pushToTalk, key: e.code } });
  }
  const savedKeyProblem = $derived(pttKeyProblem({ key: '', code: settings.pushToTalk.key })); // an older saved key

  let resetting = $state<Section | null>(null);
  function reset() {
    set(RESETS[resetting!]!);
    resetting = null;
  }
</script>

{#snippet head(s: Section)}
  <div class="flex h-8 items-center justify-between gap-4">
    <h3 id="voice-{s}" class="text-base font-medium text-fg">{NAMES[s]}</h3>
    {#if RESETS[s]}<span class="-mr-2"><Button variant="ghost" size="sm" aria-describedby="voice-{s}" onclick={() => (resetting = s)}>Reset to defaults</Button></span>{/if}
  </div>
{/snippet}

<div class="sections">
  <section role="group" aria-labelledby="voice-input">
    {@render head('input')}
    <MicCheck {settings} onchange={updateSettings} liveStream={voice.mic} bind:devices />
  </section>

  <section role="group" aria-labelledby="voice-noise">
    {@render head('noise')}
    <Field label="Noise suppression" help={MODES[settings.mode]}>
      <Select value={settings.mode} onchange={(e) => set({ mode: e.currentTarget.value as AudioSettings['mode'] })}>
        <option value="deepfilter">DeepFilterNet (best)</option>
        <option value="browser">Browser built-in</option>
        <option value="off">Off</option>
      </Select>
    </Field>
    {#if settings.mode === 'deepfilter'}
      <Field label="Attenuation limit" value="{settings.attenDb} dB" help="How far the filter may turn noise down.">
        <Range min={6} max={100} value={settings.attenDb} aria-valuetext="{settings.attenDb} dB"
          oninput={(e) => set({ attenDb: +e.currentTarget.value })} />
      </Field>
      <Field label="Post filter" value={settings.postFilter.toFixed(3)} help="Extra clean up after the filter. Higher can sound thinner.">
        <Range min={0} max={0.05} step={0.005} value={settings.postFilter} aria-valuetext={settings.postFilter.toFixed(3)}
          oninput={(e) => set({ postFilter: +e.currentTarget.value })} />
      </Field>
      <Field label="Input boost" value="{settings.inputBoostDb} dB" help="Boost lifts the mic before the noise filter. High values can clip.">
        <Range min={0} max={12} value={settings.inputBoostDb} aria-valuetext="{settings.inputBoostDb} dB"
          oninput={(e) => set({ inputBoostDb: +e.currentTarget.value })} />
      </Field>
    {/if}
    <div class="flex flex-col">
      <Switch label="Echo cancellation (always on)" checked disabled />
      <Switch label="Browser noise suppression on top" checked={settings.noiseSuppression}
        onchange={(e) => set({ noiseSuppression: e.currentTarget.checked })} />
      <Switch label="Auto gain" help="The browser evens out your volume." checked={settings.autoGain}
        onchange={(e) => set({ autoGain: e.currentTarget.checked })} />
    </div>
  </section>

  <section role="group" aria-labelledby="voice-output">
    {@render head('output')}
    {#if sinkSupported}
      <Field label="Speaker">
        <Select value={settings.speakerId ?? ''} onchange={(e) => set({ speakerId: e.currentTarget.value || null })}>
          <option value="">System default</option>
          {#each speakers as d (d.deviceId)}<option value={d.deviceId}>{d.label || d.deviceId}</option>{/each}
        </Select>
      </Field>
    {:else}
      <div class="flex flex-col gap-1.5">
        <div class="text-sm font-medium text-fg">Speaker</div>
        <p class="text-fg-2">–</p>
        <p class="text-xs text-fg-3">Your browser cannot pick a speaker. Sound plays on the system output.</p>
      </div>
    {/if}
  </section>

  <section role="group" aria-labelledby="voice-quality">
    {@render head('quality')}
    <Field label="Bitrate" value="{bitrateDraft} kbps" help="Higher sounds better and uses more data. 64 kbps is plenty for speech.">
      <Range min={32} max={510} value={settings.bitrateKbps} aria-valuetext="{bitrateDraft} kbps"
        oninput={(e) => (bitrateDraft = +e.currentTarget.value)}
        onchange={(e) => set({ bitrateKbps: +e.currentTarget.value })} />
    </Field>
    {#if voice.joined}
      <p class="text-xs text-fg-2">This call uses {Math.min(settings.bitrateKbps, voice.joinedBitrate)} kbps.</p>
      {#if bitrateDraft > voice.joinedBitrate}
        <p class="text-xs text-fg-3">Lowering applies now; raising above the value you joined with ({voice.joinedBitrate} kbps) applies on rejoin.</p>
      {/if}
    {/if}
    <Switch label="DTX (silence suppression)" help="Sends almost nothing while you are quiet. Applies at the next join." checked={settings.dtx}
      onchange={(e) => set({ dtx: e.currentTarget.checked })} />
  </section>

  <section role="group" aria-labelledby="voice-ptt">
    {@render head('ptt')}
    <Switch label="Push to talk" help="Your mic sends only while you hold the key." checked={settings.pushToTalk.enabled}
      onchange={(e) => set({ pushToTalk: { ...settings.pushToTalk, enabled: e.currentTarget.checked } })} />
    {#if settings.pushToTalk.enabled}
      <div class="flex flex-col gap-1.5">
        <div class="flex min-h-8 items-center justify-between gap-4">
          <span id="ptt-key" class="text-sm text-fg">Key</span>
          <span class:capturing><Button aria-describedby="ptt-key ptt-capture ptt-saved ptt-note" onclick={() => ([capturing, refused] = [true, ''])} onblur={() => ([capturing, refused] = [false, ''])}
            onkeydown={(e) => { if (capturing) { e.stopPropagation(); captureKey(e); } }}
            onkeyup={(e) => { if (capturing) e.stopPropagation(); }}>
            {#if capturing}Press a key...{:else}Key: <Kbd>{keyName(settings.pushToTalk.key)}</Kbd>{/if}
          </Button></span>
        </div>
        <p id="ptt-capture" role="status" class="text-xs text-fg-2 empty:sr-only">{#if capturing}Press the key to use. Esc cancels.{/if}</p>
        {#if savedKeyProblem && !capturing}<p id="ptt-saved" class="text-xs text-warn">Your saved key can get in the way. {savedKeyProblem}</p>{/if}
        {#if refused}<p class="text-xs text-warn" role="alert">{refused}</p>{/if}
        <p id="ptt-note" class="text-xs text-fg-3">Works only while this tab has focus. Browsers do not allow keys from other apps.</p>
      </div>
    {/if}
  </section>

  <section role="group" aria-labelledby="voice-keys">
    <div class="flex h-8 items-center"><h3 id="voice-keys" class="text-base font-medium text-fg">Keyboard shortcuts</h3></div>
    <ShortcutList />
  </section>
</div>

<Dialog open={resetting !== null} title="Reset {resetting ? NAMES[resetting] : ''} to defaults?" onclose={() => (resetting = null)}>
  Every setting under {resetting ? NAMES[resetting] : ''} goes back to its default. Your microphone, your speaker and the other sections stay as they are.
  {#snippet footer()}
    <Button variant="ghost" onclick={() => (resetting = null)}>Cancel</Button>
    <Button variant="danger" onclick={reset}>Reset</Button>
  {/snippet}
</Dialog>

<style>
  .sections { display: flex; flex-direction: column; gap: 32px; }
  section { display: flex; flex-direction: column; gap: 16px; }
  section + section { padding-top: 24px; border-top: 1px solid var(--line-subtle); }
  .capturing :global(.btn) { border-color: var(--accent); color: var(--accent-fg); } /* listening for a key */
</style>
