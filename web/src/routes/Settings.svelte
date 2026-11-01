<script lang="ts">
  import { clampSettings, type AudioSettings } from '../lib/audio/settings';
  import MicCheck from './settings/MicCheck.svelte';

  let { settings, onchange, liveStream, joinedBitrate, joined = false, sinkSupported = true }: {
    settings: AudioSettings;
    onchange: (s: AudioSettings) => void;
    /** The call's capture stream, metered when no mic test runs. */
    liveStream?: MediaStream;
    joinedBitrate?: number;
    joined?: boolean;
    sinkSupported?: boolean;
  } = $props();

  const set = (patch: Partial<AudioSettings>) => onchange(clampSettings({ ...settings, ...patch }));

  let bitrateDraft = $state(64); // applied on release, not on every drag step
  $effect(() => void (bitrateDraft = settings.bitrateKbps));
  let devices = $state<MediaDeviceInfo[]>([]); // filled by MicCheck
  const speakers = $derived(devices.filter((d) => d.kind === 'audiooutput'));

  let capturingKey = $state(false);
  function captureKey(e: KeyboardEvent) {
    e.preventDefault();
    capturingKey = false;
    set({ pushToTalk: { ...settings.pushToTalk, key: e.code } });
    (e.currentTarget as HTMLElement).blur(); // keep focus off the button so the PTT key reaches the window handler
  }

  const label = 'block text-sm text-fg-2';
</script>

<section class="space-y-4 rounded bg-element p-4 text-sm" aria-label="Audio settings">
  <label class={label}>Noise suppression
    <select class="w-full rounded bg-hover p-1" value={settings.mode}
      onchange={(e) => set({ mode: e.currentTarget.value as AudioSettings['mode'] })}>
      <option value="deepfilter">DeepFilterNet (best)</option>
      <option value="browser">Browser built-in</option>
      <option value="off">Off</option>
    </select>
  </label>

  {#if settings.mode === 'deepfilter'}
    <label class={label}>Attenuation limit: {settings.attenDb} dB
      <input class="w-full" type="range" min="6" max="100" value={settings.attenDb}
        oninput={(e) => set({ attenDb: +e.currentTarget.value })} />
    </label>
    <label class={label}>Post filter: {settings.postFilter.toFixed(3)}
      <input class="w-full" type="range" min="0" max="0.05" step="0.005" value={settings.postFilter}
        oninput={(e) => set({ postFilter: +e.currentTarget.value })} />
    </label>
    <label class={label}>Input boost: {settings.inputBoostDb} dB
      <input class="w-full" type="range" min="0" max="12" value={settings.inputBoostDb}
        oninput={(e) => set({ inputBoostDb: +e.currentTarget.value })} />
    </label>
    <p class="text-fg-3">Boost lifts the mic before the noise filter. High values can clip.</p>
  {/if}

  <label class={label}>Bitrate: {bitrateDraft} kbps
    <input class="w-full" type="range" min="32" max="510" value={settings.bitrateKbps}
      oninput={(e) => (bitrateDraft = +e.currentTarget.value)}
      onchange={(e) => set({ bitrateKbps: +e.currentTarget.value })} />
  </label>
  {#if joined && joinedBitrate !== undefined && bitrateDraft > joinedBitrate}
    <p class="text-fg-3">Lowering applies now; raising above the value you joined with ({joinedBitrate} kbps) applies on rejoin.</p>
  {/if}
  <label class="flex items-center gap-2"><input type="checkbox" checked={settings.dtx}
    onchange={(e) => set({ dtx: e.currentTarget.checked })} /> DTX (silence suppression; applies at next join)</label>

  <div class="space-y-1">
    <label class="flex items-center gap-2"><input type="checkbox" checked disabled /> Echo cancellation (always on)</label>
    <label class="flex items-center gap-2"><input type="checkbox" checked={settings.noiseSuppression}
      onchange={(e) => set({ noiseSuppression: e.currentTarget.checked })} /> Browser noise suppression on top</label>
    <label class="flex items-center gap-2"><input type="checkbox" checked={settings.autoGain}
      onchange={(e) => set({ autoGain: e.currentTarget.checked })} /> Auto gain</label>
  </div>

  {#if sinkSupported}
    <label class={label}>Speaker
      <select class="w-full rounded bg-hover p-1" value={settings.speakerId ?? ''}
        onchange={(e) => set({ speakerId: e.currentTarget.value || null })}>
        <option value="">System default</option>
        {#each speakers as d (d.deviceId)}<option value={d.deviceId}>{d.label || d.deviceId}</option>{/each}
      </select>
    </label>
  {:else}
    <p class="text-fg-3">Speaker choice is not supported in this browser; use the system output.</p>
  {/if}

  <MicCheck {settings} {onchange} {liveStream} bind:devices />

  <div class="space-y-1">
    <label class="flex items-center gap-2"><input type="checkbox" checked={settings.pushToTalk.enabled}
      onchange={(e) => set({ pushToTalk: { ...settings.pushToTalk, enabled: e.currentTarget.checked } })} /> Push to talk</label>
    {#if settings.pushToTalk.enabled}
      <button class="rounded bg-selected px-3 py-1" onclick={() => (capturingKey = true)}
        onkeydown={(e) => { if (capturingKey) { e.stopPropagation(); captureKey(e); } }}
        onkeyup={(e) => { if (capturingKey) e.stopPropagation(); }}>
        {capturingKey ? 'Press a key...' : `Key: ${settings.pushToTalk.key}`}
      </button>
      <p class="text-fg-3">
        Works only while this tab is focused: browsers cannot capture global hotkeys.
      </p>
    {/if}
  </div>
</section>
