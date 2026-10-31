<script lang="ts">
  import { onMount } from 'svelte';
  import { createCapture, type Capture } from '../lib/audio/capture';
  import { clampSettings, type AudioSettings } from '../lib/audio/settings';
  import { DEVICE_MESSAGES, classifyMediaError, isBluetoothLabel, levelHint, watchDevices } from '../lib/devices';

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
  let devices = $state<MediaDeviceInfo[]>([]);
  const mics = $derived(devices.filter((d) => d.kind === 'audioinput'));
  const speakers = $derived(devices.filter((d) => d.kind === 'audiooutput'));
  const micLabel = $derived(mics.find((d) => d.deviceId === (settings.micId ?? 'default'))?.label ?? mics[0]?.label ?? '');

  let test: Capture | undefined;
  let testing = $state(false);
  let testError = $state('');
  let loop: HTMLAudioElement | undefined;
  let levelDb = $state(-100);
  let peaks: number[] = [];
  let hint = $state<'clipping' | 'too-quiet' | 'ok'>('ok');

  const refresh = () => navigator.mediaDevices.enumerateDevices().then((l) => (devices = l), () => {});
  onMount(() => {
    void refresh();
    const stop = watchDevices((l) => (devices = l));
    // Meter: re-tap whenever the stream's track is swapped (mode or device change).
    let ctx: AudioContext | undefined;
    let src: MediaStreamAudioSourceNode | undefined;
    let tapped = '';
    const an = { node: undefined as AnalyserNode | undefined };
    const buf = new Float32Array(1024);
    const t = setInterval(() => {
      const stream = test?.stream ?? liveStream;
      const track = stream?.getAudioTracks()[0];
      if (!stream || !track) return void (levelDb = -100);
      ctx ??= new AudioContext();
      if (tapped !== track.id) {
        src?.disconnect();
        src = ctx.createMediaStreamSource(new MediaStream([track]));
        an.node = ctx.createAnalyser();
        an.node.fftSize = 1024;
        src.connect(an.node);
        tapped = track.id;
      }
      an.node!.getFloatTimeDomainData(buf);
      const peak = buf.reduce((m, v) => Math.max(m, Math.abs(v)), 0);
      levelDb = peak > 0 ? 20 * Math.log10(peak) : -100;
      peaks.push(levelDb);
      if (peaks.length > 30) peaks.shift();
      hint = peaks.length >= 30 ? levelHint(peaks) : 'ok';
    }, 100);
    return () => {
      stop();
      clearInterval(t);
      void ctx?.close();
      stopTest();
    };
  });

  async function startTest() {
    testError = '';
    try {
      // Same pipeline as the call, so this is what others will hear.
      test = await createCapture(settings);
      loop = new Audio();
      loop.srcObject = test.stream;
      void loop.play().catch(() => {});
      testing = true;
      void refresh(); // labels appear once permission is granted
    } catch (e) {
      testError = DEVICE_MESSAGES[classifyMediaError(e)];
    }
  }
  function stopTest() {
    if (loop) loop.srcObject = null;
    loop = undefined;
    test?.close();
    test = undefined;
    testing = false;
    peaks = [];
  }

  let capturingKey = $state(false);
  function captureKey(e: KeyboardEvent) {
    e.preventDefault();
    capturingKey = false;
    set({ pushToTalk: { ...settings.pushToTalk, key: e.code } });
    (e.currentTarget as HTMLElement).blur(); // keep focus off the button so the PTT key reaches the window handler
  }

  const pct = $derived(Math.max(0, Math.min(100, (levelDb + 100))));
  const label = 'block text-sm text-neutral-300';
</script>

<section class="space-y-4 rounded bg-neutral-900 p-4 text-sm" aria-label="Audio settings">
  <label class={label}>Noise suppression
    <select class="w-full rounded bg-neutral-800 p-1" value={settings.mode}
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
      <input class="w-full" type="range" min="0" max="20" value={settings.inputBoostDb}
        oninput={(e) => set({ inputBoostDb: +e.currentTarget.value })} />
    </label>
  {/if}

  <label class={label}>Bitrate: {bitrateDraft} kbps
    <input class="w-full" type="range" min="32" max="510" value={settings.bitrateKbps}
      oninput={(e) => (bitrateDraft = +e.currentTarget.value)}
      onchange={(e) => set({ bitrateKbps: +e.currentTarget.value })} />
  </label>
  {#if joined && joinedBitrate !== undefined && bitrateDraft > joinedBitrate}
    <p class="text-neutral-500">Lowering applies now; raising above the value you joined with ({joinedBitrate} kbps) applies on rejoin.</p>
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

  <label class={label}>Microphone
    <select class="w-full rounded bg-neutral-800 p-1" value={settings.micId ?? ''}
      onchange={(e) => set({ micId: e.currentTarget.value || null })}>
      <option value="">System default</option>
      {#each mics as d (d.deviceId)}<option value={d.deviceId}>{d.label || d.deviceId}</option>{/each}
    </select>
  </label>
  <p class="text-neutral-500">Pick a specific microphone to get disconnect alerts; the system default is not watched.</p>
  {#if isBluetoothLabel(micLabel)}
    <p class="text-amber-400" role="alert">
      Bluetooth headset microphones switch to a low-quality call profile. Use a wired or USB mic for the best sound.
    </p>
  {/if}

  {#if sinkSupported}
    <label class={label}>Speaker
      <select class="w-full rounded bg-neutral-800 p-1" value={settings.speakerId ?? ''}
        onchange={(e) => set({ speakerId: e.currentTarget.value || null })}>
        <option value="">System default</option>
        {#each speakers as d (d.deviceId)}<option value={d.deviceId}>{d.label || d.deviceId}</option>{/each}
      </select>
    </label>
  {:else}
    <p class="text-neutral-500">Speaker choice is not supported in this browser; use the system output.</p>
  {/if}

  <div class="space-y-1">
    <div class={label}>Input level; marker at {settings.vad.thresholdDb} dB (voice-activity gating is not implemented yet, display only)</div>
    <div class="relative h-3 rounded bg-neutral-800" role="meter" aria-label="Input level" aria-valuemin="-100"
      aria-valuemax="0" aria-valuenow={Math.round(levelDb)}>
      <div class="h-3 rounded bg-green-600" style="width:{pct}%"></div>
      <div class="absolute top-0 h-3 w-0.5 bg-white" style="left:{settings.vad.thresholdDb + 100}%"></div>
    </div>
    <input class="w-full" type="range" min="-100" max="0" aria-label="Level marker" value={settings.vad.thresholdDb}
      oninput={(e) => set({ vad: { thresholdDb: +e.currentTarget.value } })} />
    {#if hint === 'clipping'}<p class="text-amber-400">Too loud: the signal is clipping. Lower input gain.</p>{/if}
    {#if hint === 'too-quiet'}<p class="text-amber-400">Very quiet or silent. Speak, raise the input boost, or check the mic.</p>{/if}
  </div>

  <div class="space-y-1">
    <button class="rounded bg-neutral-700 px-3 py-1" onclick={() => (testing ? stopTest() : startTest())}>
      {testing ? 'Stop mic test' : 'Mic test (hear yourself)'}
    </button>
    <span class="text-neutral-500"> Use headphones, or you will get feedback.</span>
    {#if testError}<p class="text-red-400" role="alert">{testError}</p>{/if}
  </div>

  <div class="space-y-1">
    <label class="flex items-center gap-2"><input type="checkbox" checked={settings.pushToTalk.enabled}
      onchange={(e) => set({ pushToTalk: { ...settings.pushToTalk, enabled: e.currentTarget.checked } })} /> Push to talk</label>
    {#if settings.pushToTalk.enabled}
      <button class="rounded bg-neutral-700 px-3 py-1" onclick={() => (capturingKey = true)}
        onkeydown={(e) => { if (capturingKey) { e.stopPropagation(); captureKey(e); } }}
        onkeyup={(e) => { if (capturingKey) e.stopPropagation(); }}>
        {capturingKey ? 'Press a key...' : `Key: ${settings.pushToTalk.key}`}
      </button>
      <p class="text-neutral-500">
        Works only while this tab is focused: browsers cannot capture global hotkeys.
      </p>
    {/if}
  </div>
</section>
