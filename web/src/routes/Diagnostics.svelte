<script lang="ts">
  import { onMount } from 'svelte';
  import { connectivityCheck, summarize, type Diag } from '../lib/diag';

  // The key is used only for the connectivity check and never appears in the report.
  let { pc, wsUrl, joinKey, mode, status }: { pc?: RTCPeerConnection; wsUrl: string; joinKey: string; mode: string; status: string } = $props();
  let diag = $state<Diag | null>(null);
  let result = $state('');
  let testing = $state(false);
  let copied = $state('');

  onMount(() => {
    let prev: RTCStatsReport | null = null;
    let at = 0;
    const tick = async () => {
      if (!pc) { diag = null; prev = null; return; }
      try {
        const cur = await pc.getStats();
        const now = performance.now();
        diag = summarize(prev, cur, (now - at) / 1000);
        prev = cur;
        at = now;
      } catch { diag = null; }
    };
    void tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
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
      `Loss: ${f(d?.lossPct ?? null, '%')}`,
      `Send: ${f(d?.sendKbps ?? null, 'kbps')}`,
      `Codec: ${d?.codec ?? 'n/a'}`,
      `ICE state: ${d?.iceState ?? 'n/a'}`,
      `Selected pair: ${d?.selectedPair ?? 'n/a'}`,
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
    const r = await connectivityCheck(wsUrl, joinKey);
    result = `${r.ok ? 'OK' : 'FAILED'} in ${r.ms} ms: ${r.reason}`;
    testing = false;
  }
</script>

<section class="space-y-2 rounded bg-neutral-900 p-3 text-sm">
  <dl class="grid grid-cols-2 gap-x-2">
    <dt>RTT</dt><dd>{f(diag?.rttMs ?? null, 'ms')}</dd>
    <dt>Jitter</dt><dd>{f(diag?.jitterMs ?? null, 'ms')}</dd>
    <dt>Loss</dt><dd>{f(diag?.lossPct ?? null, '%')}</dd>
    <dt>Send</dt><dd>{f(diag?.sendKbps ?? null, 'kbps')}</dd>
    <dt>Codec</dt><dd>{diag?.codec ?? 'n/a'}</dd>
    <dt>ICE</dt><dd>{diag?.iceState ?? 'n/a'}</dd>
    <dt>Path</dt><dd>{diag?.selectedPair ?? 'n/a'}</dd>
  </dl>
  <div class="flex gap-2">
    <button class="rounded bg-neutral-700 px-3 py-1" onclick={copy}>Copy report</button>
    <button class="rounded bg-neutral-700 px-3 py-1" onclick={test} disabled={testing || !joinKey}>
      {testing ? 'Testing...' : 'Test connection'}
    </button>
  </div>
  {#if !joinKey}<p class="text-neutral-400">Enter the key above to test the connection.</p>{/if}
  {#if copied}<p role="status">{copied}</p>{/if}
  {#if result}<p role="status">{result}</p>{/if}
</section>
