<script lang="ts">
  import { onMount } from 'svelte';
  import { createCapture, type Capture } from '../../lib/audio/capture';
  import { clampSettings, type AudioSettings } from '../../lib/audio/settings';
  import { MIC_FALLBACK, hintLevelDb, isBluetoothLabel, levelHint, micStartError, watchDevices } from '../../lib/devices';
  import Button from '../../lib/ui/c/Button.svelte';
  import Field from '../../lib/ui/c/Field.svelte';
  import Meter from '../../lib/ui/c/Meter.svelte';
  import Range from '../../lib/ui/c/Range.svelte';
  import Select from '../../lib/ui/c/Select.svelte';
  import Switch from '../../lib/ui/c/Switch.svelte';
  // The microphone picker, the input level meter and the mic test, for Settings > Voice and the welcome steps.
  // `devices` is bindable so Settings can list the speakers from the same list. `marker` shows the level marker slider.
  let { settings, onchange, liveStream, liveCapture, marker = true, devices = $bindable([]) }: {
    settings: AudioSettings;
    onchange: (s: AudioSettings) => void;
    /** The call's capture stream, metered when no mic test runs. */
    liveStream?: MediaStream;
    /** The call's capture, for the pre-limiter level behind the clipping hint. */
    liveCapture?: Capture;
    marker?: boolean;
    devices?: MediaDeviceInfo[];
  } = $props();

  const set = (patch: Partial<AudioSettings>) => onchange(clampSettings({ ...settings, ...patch }));
  const mics = $derived(devices.filter((d) => d.kind === 'audioinput'));
  const micLabel = $derived(mics.find((d) => d.deviceId === (settings.micId ?? 'default'))?.label ?? mics[0]?.label ?? '');

  let test: Capture | undefined;
  let testing = $state(false);
  let testError = $state('');
  let testNotice = $state('');
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
      peaks.push(hintLevelDb(levelDb, (test ?? liveCapture)?.getStats()));
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
    testError = testNotice = '';
    try {
      // Same pipeline as the call, so this is what others will hear. In a call, the call's own stream: no second mic.
      if (!liveStream) test = await createCapture(settings, { onMicFallback: () => (set({ micId: null }), (testNotice = MIC_FALLBACK)) });
      loop = new Audio();
      loop.srcObject = test?.stream ?? liveStream!;
      void loop.play().catch(() => {});
      testing = true;
      void refresh(); // labels appear once permission is granted
    } catch (e) {
      testError = micStartError(e);
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
  // A test on the call's stream follows the call: it stops when the call ends and follows a new stream.
  $effect(() => {
    const live = liveStream;
    if (!testing || test || !loop) return;
    if (!live) stopTest();
    else if (loop.srcObject !== live) loop.srcObject = live;
  });

</script>

<Field label="Microphone" help="Pick a microphone by name to get an alert when it is unplugged. The system default is not watched.">
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

<div class="flex flex-col gap-1.5">
  <div class="text-sm font-ui text-fg" aria-hidden="true">Input level</div>
  <Meter db={levelDb} marker={marker ? settings.vad.thresholdDb : undefined} />
  {#if hint === 'clipping'}<p class="text-xs text-warn">Too loud: the signal is clipping. Lower input gain.</p>{/if}
  {#if hint === 'too-quiet'}<p class="text-xs text-warn">Very quiet or silent. Speak, check the mic, or try a little input boost.</p>{/if}
</div>
{#if marker}
  <Field label="Level marker" value="{settings.vad.thresholdDb} dB" help="The white line on the meter. With the noise gate on, sound below it is muted.">
    <Range min={-100} max={0} aria-valuetext="{settings.vad.thresholdDb} dB" value={settings.vad.thresholdDb}
      oninput={(e) => set({ vad: { ...settings.vad, thresholdDb: +e.currentTarget.value } })} />
  </Field>
  <Switch label="Noise gate" checked={settings.vad.gate} disabled={settings.mode !== 'deepfilter'}
    help={settings.mode === 'deepfilter'
      ? 'Mutes sound under the marker, like fan hiss between words. It mutes about 0.2 s after you stop. Turn it off while you set the marker: the meter shows the gated sound.'
      : 'Works only with DeepFilterNet noise suppression.'}
    onchange={(e) => set({ vad: { ...settings.vad, gate: e.currentTarget.checked } })} />
  <Switch label="Automatic threshold" checked={settings.vad.auto} disabled={settings.mode !== 'deepfilter' || !settings.vad.gate}
    help="The gate ignores the marker and follows your background noise: it mutes sound under 3 times its level over the last 2 s. Off by default."
    onchange={(e) => set({ vad: { ...settings.vad, auto: e.currentTarget.checked } })} />
{/if}

<div class="flex flex-wrap items-center gap-x-3 gap-y-1">
  <Button onclick={() => (testing ? stopTest() : startTest())}>
    {testing ? 'Stop mic test' : 'Mic test (hear yourself)'}
  </Button>
  <span class="text-xs text-fg-3">Use headphones, or you will get feedback.</span>
  {#if testError}<p class="w-full text-danger" role="alert">{testError}</p>{/if}
  {#if testNotice}<p class="w-full text-warn" role="status">{testNotice}</p>{/if}
</div>
