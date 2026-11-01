<script lang="ts">
  import { onMount } from 'svelte';
  import { createCapture, type Capture } from '../../lib/audio/capture';
  import { clampSettings, type AudioSettings } from '../../lib/audio/settings';
  import { DEVICE_MESSAGES, classifyMediaError, isBluetoothLabel, levelHint, watchDevices } from '../../lib/devices';
  import Button from '../../lib/ui/c/Button.svelte';
  import Field from '../../lib/ui/c/Field.svelte';
  import Select from '../../lib/ui/c/Select.svelte';
  // The microphone picker, the input level meter and the mic test, for Settings > Voice and the welcome steps.
  // `devices` is bindable so Settings can list the speakers from the same list. `marker` shows the level marker slider.
  let { settings, onchange, liveStream, marker = true, devices = $bindable([]) }: {
    settings: AudioSettings;
    onchange: (s: AudioSettings) => void;
    /** The call's capture stream, metered when no mic test runs. */
    liveStream?: MediaStream;
    marker?: boolean;
    devices?: MediaDeviceInfo[];
  } = $props();

  const set = (patch: Partial<AudioSettings>) => onchange(clampSettings({ ...settings, ...patch }));
  const mics = $derived(devices.filter((d) => d.kind === 'audioinput'));
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

  const pct = $derived(Math.max(0, Math.min(100, levelDb + 100)));
  const label = 'block text-sm text-fg-2';
</script>

<Field label="Microphone" help="Pick a specific microphone to get disconnect alerts; the system default is not watched.">
  <Select value={settings.micId ?? ''} onchange={(e) => set({ micId: e.currentTarget.value || null })}>
    <option value="">System default</option>
    {#each mics as d (d.deviceId)}<option value={d.deviceId}>{d.label || d.deviceId}</option>{/each}
  </Select>
</Field>
{#if isBluetoothLabel(micLabel)}
  <p class="text-warn" role="alert">
    Bluetooth headset microphones switch to a low-quality call profile. Use a wired or USB mic for the best sound.
  </p>
{/if}

<div class="space-y-1">
  {#if marker}
    <div class={label}>Input level; marker at {settings.vad.thresholdDb} dB (voice-activity gating is not implemented yet, display only)</div>
  {:else}
    <div class={label} aria-hidden="true">Input level</div>
  {/if}
  <div class="relative h-3 rounded border border-line bg-app" role="meter" aria-label="Input level" aria-valuemin="-100"
    aria-valuemax="0" aria-valuenow={Math.round(levelDb)}>
    <div class="h-full rounded bg-ok" style="width:{pct}%"></div>
    {#if marker}<div class="absolute top-0 h-full w-0.5 bg-fg" style="left:{settings.vad.thresholdDb + 100}%"></div>{/if}
  </div>
  {#if marker}
    <input class="w-full" type="range" min="-100" max="0" aria-label="Level marker" value={settings.vad.thresholdDb}
      oninput={(e) => set({ vad: { thresholdDb: +e.currentTarget.value } })} />
  {/if}
  {#if hint === 'clipping'}<p class="text-warn">Too loud: the signal is clipping. Lower input gain.</p>{/if}
  {#if hint === 'too-quiet'}<p class="text-warn">Very quiet or silent. Speak, check the mic, or try a little input boost.</p>{/if}
</div>

<div class="flex flex-wrap items-center gap-x-3 gap-y-1">
  <Button onclick={() => (testing ? stopTest() : startTest())}>
    {testing ? 'Stop mic test' : 'Mic test (hear yourself)'}
  </Button>
  <span class="text-fg-3">Use headphones, or you will get feedback.</span>
  {#if testError}<p class="w-full text-danger" role="alert">{testError}</p>{/if}
</div>
