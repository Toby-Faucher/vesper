<script lang="ts">
  import { connectivityCheck, summarize, type Diag } from '../lib/diag';
  import { audioRows } from '../lib/audio/level';
  import type { Capture, CaptureStats } from '../lib/audio/capture';

  import { copyText } from '../lib/ui/clipboard';
  import Button from '../lib/ui/c/Button.svelte';
  // Settings > Diagnostics: the call's numbers, the audio pipeline, a copyable report and the connection test.
  let { pc, capture, wsUrl, mode, reason = '', status }: {
    pc?: RTCPeerConnection; capture?: Capture; wsUrl: string; mode: string; reason?: string; status: string;
  } = $props();
  const noise = $derived(`Noise suppression: ${mode}${reason ? ` (${reason})` : ''}`);
  let diag = $state<Diag | null>(null);
  let audio = $state.raw<CaptureStats | null>(null);
  const rows = $derived(audioRows(capture ? audio : null));
  let result = $state('');
  let testing = $state(false);
  let copied = $state('');

  $effect(() => {
    const conn = pc; // re-runs when the peer connection changes; prev resets with it
    diag = null;
    audio = null;
    if (!conn) return;
    let prev: RTCStatsReport | null = null;
    let at = 0;
    let busy = false;
    let dead = false;
    const tick = async () => {
      if (busy) return;
      busy = true;
      audio = capture?.getStats() ?? null;
      try {
        const cur = await conn.getStats();
        if (dead) return;
        const now = performance.now();
        diag = summarize(prev, cur, (now - at) / 1000, conn.iceConnectionState);
        prev = cur;
        at = now;
      } catch {
        if (!dead) diag = null;
      } finally {
        busy = false;
      }
    };
    void tick();
    const id = setInterval(tick, 1000);
    return () => {
      dead = true;
      clearInterval(id);
    };
  });

  const f = (v: number | null, u: string) => (v === null ? 'n/a' : `${v.toFixed(1)} ${u}`);
  const show = (v: string) => (v === 'n/a' ? '–' : v); // the page shows a dash, the report keeps n/a
  function report(): string {
    const d = diag;
    return [
      `Vesper diagnostics ${new Date().toISOString()}`,
      `User agent: ${navigator.userAgent}`,
      `Mode: ${mode}`,
      noise,
      `Status: ${status}`,
      `RTT: ${f(d?.rttMs ?? null, 'ms')}`,
      `Jitter: ${f(d?.jitterMs ?? null, 'ms')}`,
      `Loss (incoming): ${f(d?.lossDownPct ?? null, '%')}`,
      `Loss (outgoing): ${f(d?.lossUpPct ?? null, '%')}`,
      `Send: ${f(d?.sendKbps ?? null, 'kbps')}`,
      `Codec: ${d?.codec ?? 'n/a'}`,
      `ICE state: ${d?.iceState ?? 'n/a'}`,
      `Selected pair: ${d?.selectedPair ?? 'n/a'}`,
      'Audio pipeline:',
      ...rows.map(([k, v]) => `  ${k}: ${v}`),
      result ? `Connection test: ${result}` : '',
    ].filter(Boolean).join('\n');
  }
  async function copy() {
    copied = (await copyText(report())) ? 'Copied.' : 'Could not copy. This browser blocked the clipboard.';
  }
  async function test() {
    testing = true;
    result = '';
    const r = await connectivityCheck(wsUrl);
    result = `${r.ok ? 'OK' : 'FAILED'} in ${r.ms} ms: ${r.reason}`;
    testing = false;
  }
</script>

{#snippet rows2(list: [string, string][])}
  <dl class="grid grid-cols-[minmax(0,12rem)_1fr] gap-x-6 gap-y-1 tabular-nums">
    {#each list as [k, v] (k)}<dt class="text-fg-2">{k}</dt><dd class="min-w-0 break-words text-fg">{show(v)}</dd>{/each}
  </dl>
{/snippet}

<div class="flex flex-col gap-8">
  <section role="group" aria-labelledby="diag-call" class="flex flex-col gap-3">
    <h3 id="diag-call" class="text-base font-medium text-fg">Call numbers</h3>
    <p class="text-fg">{noise}</p>
    {#if !pc}<p class="text-xs text-fg-3">Not in a call. A dash means no value: join voice in a room to see the numbers.</p>
    {:else if !diag}<p class="text-xs text-fg-3">Waiting for the first numbers from the call.</p>
    {:else}<p class="text-xs text-fg-3">A dash means no value yet. Jitter and incoming loss need sound from someone else in the call.</p>{/if}
    {@render rows2([
      ['RTT', f(diag?.rttMs ?? null, 'ms')],
      ['Jitter', f(diag?.jitterMs ?? null, 'ms')],
      ['Loss (incoming)', f(diag?.lossDownPct ?? null, '%')],
      ['Loss (outgoing)', f(diag?.lossUpPct ?? null, '%')],
      ['Send', f(diag?.sendKbps ?? null, 'kbps')],
      ['Codec', diag?.codec ?? 'n/a'],
      ['ICE', diag?.iceState ?? 'n/a'],
      ['Path', diag?.selectedPair ?? 'n/a'],
    ])}
  </section>
  <section role="group" aria-labelledby="diag-audio" class="flex flex-col gap-3 border-t border-subtle pt-6">
    <h3 id="diag-audio" class="text-base font-medium text-fg">Audio pipeline</h3>
    {@render rows2(rows)}
  </section>
  <section role="group" aria-labelledby="diag-tools" class="flex flex-col gap-3 border-t border-subtle pt-6">
    <h3 id="diag-tools" class="text-base font-medium text-fg">Report and test</h3>
    <p class="text-xs text-fg-3">Test connection checks the server and the UDP path the call uses. Copy report copies this page and your browser version, to paste into a bug report.</p>
    <div class="flex gap-2">
      <Button onclick={copy}>Copy report</Button>
      <Button onclick={test} loading={testing}>Test connection</Button>
    </div>
    {#if copied}<p role="status" class="text-fg-2">{copied}</p>{/if}
    {#if result}<p role="status" class="text-fg tabular-nums">{result}</p>{/if}
  </section>
</div>
