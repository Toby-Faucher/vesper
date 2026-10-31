<script lang="ts">
  import { Signaling, joinRoom } from '../lib/signaling';
  import type { Peer } from '../lib/gen/Peer';

  let name = $state('');
  let key = $state('');
  let peers = $state<Peer[]>([]);
  let joined = $state(false);
  let error = $state('');

  async function join(e: Event) {
    e.preventDefault();
    error = '';
    try {
      const proto = location.protocol === 'https:' ? 'wss' : 'ws';
      const sig = new Signaling(`${proto}://${location.host}/ws`);
      sig.onClose(() => (joined = false));
      sig.onMessage((m) => {
        if (m.type === 'peer_joined') peers = [...peers, m.peer];
        else if (m.type === 'peer_left') peers = peers.filter((p) => p.user_id !== m.user_id);
        else if (m.type === 'peer_muted')
          peers = peers.map((p) => (p.user_id === m.user_id ? { ...p, muted: m.muted } : p));
      });
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, channelCount: 1 },
      });
      const r = await joinRoom(sig, { room: 'main', name, key, stream, bitrateKbps: 64 });
      peers = r.peers;
      joined = true;
    } catch (err) {
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
