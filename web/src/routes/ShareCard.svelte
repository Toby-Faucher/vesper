<script lang="ts">
  import * as I from '../lib/ui/icons';
  import { shares } from '../lib/share/state.svelte';
  import { shareLine } from '../lib/share/line';
  import { stopShare, voice } from '../lib/voice.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';
  // My own share in the call bar: a muted preview, the live figures from the video sender's stats (every 2 s), the settings
  // and Stop. Nothing here is a live region: the figures change on every poll.
  let { onsettings }: { onsettings: () => void } = $props();
  const track = $derived(shares.mine?.track); // keyed on the track: a watcher count or settings change must not restart the preview or the stats
  let video: HTMLVideoElement;
  let stats = $state({ width: 0, height: 0, fps: 0, kbps: 0 });

  $effect(() => {
    if (track) video.srcObject = new MediaStream([track]);
  });
  $effect(() => {
    const sender = voice.pc?.getSenders().find((s) => s.track === track);
    if (!sender) return;
    let bytes = -1, at = 0;
    const poll = async () => {
      for (const r of (await sender.getStats().catch(() => new Map())).values()) {
        if (r.type !== 'outbound-rtp' || r.kind !== 'video') continue;
        const kbps = bytes < 0 || r.timestamp <= at ? 0 : ((r.bytesSent - bytes) * 8) / (r.timestamp - at);
        [bytes, at] = [r.bytesSent, r.timestamp];
        stats = { width: r.frameWidth ?? 0, height: r.frameHeight ?? 0, fps: r.framesPerSecond ?? 0, kbps };
      }
    };
    void poll();
    const t = setInterval(poll, 2000);
    return () => clearInterval(t);
  });
</script>

<div class="flex min-w-0 items-center gap-2">
  <video bind:this={video} class="h-8 w-14 shrink-0 rounded-sm bg-element object-contain" muted autoplay playsinline aria-hidden="true"></video>
  <span class="truncate text-xs text-fg-2 tabular-nums">{shareLine({ ...stats, watchers: shares.mine?.watchers ?? 0 })}</span>
  <IconButton icon={I.settings} label="Share settings" tip="up" onclick={onsettings} />
  <Button variant="danger-ghost" aria-label="Stop sharing" onclick={stopShare}>Stop</Button>
</div>
