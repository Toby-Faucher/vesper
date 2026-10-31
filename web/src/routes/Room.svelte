<script lang="ts">
  import { Signaling, joinRoom } from '../lib/signaling';
  import { applyPeerMsg } from '../lib/peers';
  import type { Peer } from '../lib/gen/Peer';
  import type { ServerMsg } from '../lib/gen/ServerMsg';

  let name = $state('');
  let key = $state('');
  let peers = $state<Peer[]>([]);
  let joined = $state(false);
  let error = $state('');

  async function join(e: Event) {
    e.preventDefault();
    error = '';
    let sig: Signaling | undefined;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, channelCount: 1 },
      });
      const proto = location.protocol === 'https:' ? 'wss' : 'ws';
      sig = new Signaling(`${proto}://${location.host}/ws`);
      const early: ServerMsg[] = []; // peer events that arrive before Joined
      sig.onClose(() => (joined = false));
      sig.onMessage((m) => (joined ? (peers = applyPeerMsg(peers, m)) : early.push(m)));
      const r = await joinRoom(sig, {
        room: 'main', name, key, stream, bitrateKbps: 64,
        onError: (err) => (error = err.message),
      });
      peers = early.reduce(applyPeerMsg, r.peers);
      joined = true;
    } catch (err) {
      sig?.close();
      error = String(err);
    }
  }
</script>

<main class="mx-auto max-w-sm space-y-4 p-6">
  {#if !joined}
    <form onsubmit={join} class="space-y-3">
      <input class="w-full rounded bg-neutral-900 p-2" placeholder="Name" bind:value={name} required />
      <input class="w-full rounded bg-neutral-900 p-2" placeholder="Key" type="password" bind:value={key} required />
      <button class="w-full rounded bg-neutral-700 p-2">Join</button>
    </form>
  {/if}
  {#if error}<p class="text-red-400">{error}</p>{/if}
  <ul class="space-y-1">
    {#each peers as p (p.user_id)}
      <li>{p.name}{p.muted ? ' (muted)' : ''}</li>
    {/each}
  </ul>
</main>
