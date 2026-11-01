<script lang="ts">
  import { connectivityCheck, summarize, type Diag } from '../lib/diag';
  import { audioRows } from '../lib/audio/level';
  import type { Capture, CaptureStats } from '../lib/audio/capture';

  let { pc, capture, wsUrl, mode, status }: {
    pc?: RTCPeerConnection; capture?: Capture; wsUrl: string; mode: string; status: string;
  } = $props();
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
  function report(): string {
    const d = diag;
    return [
      `Vesper diagnostics ${new Date().toISOString()}`,
      `User agent: ${navigator.userAgent}`,
      `Mode: ${mode}`,
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
    try {
      await navigator.clipboard.writeText(report());
      copied = 'Copied';
    } catch {
      copied = 'Copy failed: select the text below instead';
    }
  }
  async function test() {
    testing = true;
    result = '';
    const r = await connectivityCheck(wsUrl);
    result = `${r.ok ? 'OK' : 'FAILED'} in ${r.ms} ms: ${r.reason}`;
    testing = false;
  }
</script>

<section class="space-y-2 rounded bg-neutral-900 p-3 text-sm">
  {#if !pc}<p class="text-neutral-400">Not in a call</p>{/if}
  <dl class="grid grid-cols-2 gap-x-2">
    <dt>RTT</dt><dd>{f(diag?.rttMs ?? null, 'ms')}</dd>
    <dt>Jitter</dt><dd>{f(diag?.jitterMs ?? null, 'ms')}</dd>
    <dt>Loss (incoming)</dt><dd>{f(diag?.lossDownPct ?? null, '%')}</dd>
    <dt>Loss (outgoing)</dt><dd>{f(diag?.lossUpPct ?? null, '%')}</dd>
    <dt>Send</dt><dd>{f(diag?.sendKbps ?? null, 'kbps')}</dd>
    <dt>Codec</dt><dd>{diag?.codec ?? 'n/a'}</dd>
    <dt>ICE</dt><dd>{diag?.iceState ?? 'n/a'}</dd>
    <dt>Path</dt><dd>{diag?.selectedPair ?? 'n/a'}</dd>
  </dl>
  <h3 class="font-semibold">Audio pipeline</h3>
  <dl class="grid grid-cols-2 gap-x-2">
    {#each rows as [k, v] (k)}<dt>{k}</dt><dd>{v}</dd>{/each}
  </dl>
  <div class="flex gap-2">
    <button class="rounded bg-neutral-700 px-3 py-1" onclick={copy}>Copy report</button>
    <button class="rounded bg-neutral-700 px-3 py-1" onclick={test} disabled={testing}>
      {testing ? 'Testing...' : 'Test connection'}
    </button>
  </div>
  {#if copied}<p role="status">{copied}</p>{/if}
  {#if result}<p role="status">{result}</p>{/if}
</section>
