<script lang="ts">
  import { untrack } from 'svelte';
  import * as I from '../lib/ui/icons';
  import { eyeOff, maximize, mousePointerClick, pictureInPicture2, zoomIn } from '../lib/share/icons';
  import { session } from '../lib/session.svelte';
  import { pointAt, voice } from '../lib/voice.svelte';
  import { pictureBox, pointIn } from '../lib/share/geometry';
  import { figures } from '../lib/share/line';
  import { canPopOut, closePopOut, popOut } from '../lib/share/pip';
  import { shares } from '../lib/share/state.svelte';
  import { struggling, type RecvSample } from '../lib/share/struggle';
  import { avatarSlot } from '../lib/ui/avatar';
  import { UI_MESSAGES as M } from '../lib/ui/messages';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Icon from '../lib/ui/c/Icon.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';
  // One share in the panel. With `onpick` it is a thumbnail in Focus: no controls, and a click makes it the big one.
  let { userId, onpick }: { userId: string; onpick?: () => void } = $props();

  const user = (id: string) =>
    session.state.users[id] ?? { user_id: id, display_name: session.state.voice.find((p) => p.user_id === id)?.name ?? '', accent: '' };
  const u = $derived(user(userId));
  const stream = $derived(shares.streams.get(userId));
  const track = $derived(stream?.getVideoTracks()[0]);
  let tile = $state<HTMLElement>();
  let video = $state<HTMLVideoElement>();
  let vw = $state(0), vh = $state(0); // the video's own size
  let fw = $state(0), fh = $state(0); // the frame's size, untransformed
  const pic = $derived(pictureBox(fw, fh, vw, vh));

  $effect(() => {
    if (video) video.srcObject = stream ?? null;
  });
  // Watching: this tile counts while the panel is open and the share is not hidden.
  $effect(() => {
    if (!shares.panelOpen || shares.hidden.has(userId)) return;
    const id = userId;
    untrack(() => (shares.shown = new Set(shares.shown).add(id)));
    return () => untrack(() => {
      const s = new Set(shares.shown);
      s.delete(id);
      shares.shown = s;
    });
  });

  // Stats from this track's receiver every 2 s; the same samples say whether the share struggles.
  let line = $state('');
  let struggle = $state(false);
  $effect(() => {
    const rx = track && voice.pc?.getReceivers().find((r) => r.track === track);
    if (!rx) return;
    const samples: RecvSample[] = [];
    let bytes = -1, at = 0;
    const poll = async () => {
      for (const r of (await rx.getStats().catch(() => new Map())).values()) {
        if (r.type !== 'inbound-rtp' || r.kind !== 'video') continue;
        const kbps = bytes < 0 || r.timestamp <= at ? 0 : ((r.bytesReceived - bytes) * 8) / (r.timestamp - at);
        [bytes, at] = [r.bytesReceived, r.timestamp];
        line = r.frameWidth ? figures({ width: r.frameWidth, height: r.frameHeight, fps: r.framesPerSecond ?? 0, kbps }) : '';
        samples.push({ packetsLost: r.packetsLost ?? 0, packetsReceived: r.packetsReceived ?? 0, freezeCount: r.freezeCount ?? 0 });
        if (samples.length > 4) samples.shift();
        struggle = struggling(samples);
      }
    };
    void poll();
    const t = setInterval(poll, 2000);
    return () => clearInterval(t);
  });

  // Full screen at 1:1: one video pixel is one screen pixel, scrolled when it is larger than the screen.
  let full = $state(false);
  const toggleFull = () => (full ? document.exitFullscreen() : tile?.requestFullscreen())?.catch(() => {});

  // Zoom: 2x around transform-origin, which a click moves and a drag pans. Point: the next click inside the picture.
  let zoom = $state<{ x: number; y: number } | null>(null);
  let pointing = $state(false);
  let drag: { x: number; y: number; zx: number; zy: number } | null = null;
  let moved = false;
  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  function ondown(e: PointerEvent) {
    moved = false;
    if (!zoom || pointing) return;
    drag = { x: e.clientX, y: e.clientY, zx: zoom.x, zy: zoom.y };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onmove(e: PointerEvent) {
    if (!drag) return;
    const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
    if (Math.abs(dx) + Math.abs(dy) > 4) moved = true;
    zoom = { x: clamp(drag.zx - dx / fw), y: clamp(drag.zy - dy / fh) };
  }
  function onclick(e: MouseEvent) {
    if (pointing && video) {
      const p = pointIn(e.clientX, e.clientY, video.getBoundingClientRect(), video.videoWidth, video.videoHeight);
      if (!p) return; // outside the picture
      pointing = false;
      pointAt(userId, p.x, p.y);
    } else if (zoom && !moved) {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect(); // as drawn, so this is the spot of the share clicked
      zoom = { x: clamp((e.clientX - r.left) / r.width), y: clamp((e.clientY - r.top) / r.height) };
    }
  }

  function pop() {
    if (shares.popped === userId) return closePopOut();
    if (!stream) return;
    shares.popped = userId;
    void popOut(stream, () => (shares.popped = null));
  }
  const hide = () => (shares.hidden = new Set(shares.hidden).add(userId));
  const marks = $derived(onpick ? [] : shares.markers.filter((m) => m.share === userId));
</script>

<svelte:document onfullscreenchange={() => (full = !!tile && document.fullscreenElement === tile)} />

<div class="tile relative aspect-video border border-subtle bg-sidebar {full ? 'flex overflow-auto' : `overflow-hidden ${onpick ? 'w-44 rounded-md' : 'rounded-lg'}`}" role="group"
  aria-label={M.shareTile(u.display_name)} bind:this={tile}>
  <!-- Point and zoom clicks need a pointer; the toolbar buttons below take the keyboard. -->
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class={full ? 'relative m-auto flex-none' : 'absolute inset-0'} bind:clientWidth={fw} bind:clientHeight={fh}
    style:width={full && vw ? `${vw / devicePixelRatio}px` : null} style:height={full && vh ? `${vh / devicePixelRatio}px` : null}
    style:cursor={pointing ? 'crosshair' : zoom ? 'move' : null}
    style:transform={zoom ? 'scale(2)' : null} style:transform-origin={zoom ? `${zoom.x * 100}% ${zoom.y * 100}%` : null}
    onpointerdown={ondown} onpointermove={onmove} onpointerup={() => (drag = null)} onpointercancel={() => (drag = null)} {onclick}>
    <video class="block h-full w-full object-contain" bind:this={video} bind:videoWidth={vw} bind:videoHeight={vh} autoplay muted playsinline></video>
    {#if marks.length}
      <div class="pointer-events-none absolute" style:left="{pic.left}px" style:top="{pic.top}px" style:width="{pic.width}px" style:height="{pic.height}px" aria-hidden="true">
        {#each marks as m (m)}
          {@const who = user(m.from)}
          <span class="absolute" style:left="{m.x * 100}%" style:top="{m.y * 100}%">
            <span class="dot absolute -top-2 -left-2 size-4 rounded-full border-2 border-fg" style:background="var(--avatar-{avatarSlot(who)})"></span><span class="flex items-center gap-1.5 rounded-md bg-app text-xs text-fg absolute top-2 left-3 px-2 py-0.5 whitespace-nowrap">{who.display_name}</span>
          </span>
        {/each}
      </div>
    {/if}
  </div>

  {#if onpick}
    <button type="button" class="cover absolute inset-0 cursor-pointer" aria-label={M.shareBig} onclick={onpick}></button>
  {:else}
    {#if struggle}
      <p class="flex items-center gap-1.5 rounded-md bg-app text-xs text-fg absolute top-2 left-2 px-2 py-1" style:max-width="calc(100% - 200px)" role="status"><span class="text-warn"><Icon d={I.warn} /></span>{M.shareStruggling(u.display_name)}</p>
    {/if}
    <div class="flex items-center gap-1.5 rounded-md bg-app text-xs text-fg hov absolute top-2 right-2 gap-0.5 p-0.5" role="toolbar" aria-label={M.shareControls}>
      <IconButton icon={mousePointerClick} label={M.sharePoint} tip pressed={pointing} onclick={() => (pointing = !pointing)} />
      <IconButton icon={zoomIn} label={M.shareZoom} tip pressed={!!zoom} onclick={() => (zoom = zoom ? null : { x: 0.5, y: 0.5 })} />
      <IconButton icon={maximize} label={M.shareFull} tip pressed={full} onclick={toggleFull} />
      {#if canPopOut()}<IconButton icon={pictureInPicture2} label={M.sharePopOut} tip pressed={shares.popped === userId} onclick={pop} />{/if}
      <IconButton icon={eyeOff} label={M.shareHide} tip onclick={hide} />
    </div>
  {/if}
  <div class="pointer-events-none absolute inset-x-2 bottom-2 flex min-w-0 gap-1.5">
    <span class="flex items-center gap-1.5 rounded-md bg-app text-xs text-fg h-6 pr-2 pl-0.5 whitespace-nowrap"><Avatar user={u} size={20} />{u.display_name}</span>
    {#if line && !onpick}<span class="flex items-center gap-1.5 rounded-md bg-app text-xs text-fg hov truncate px-2 tabular-nums">{line}</span>{/if}
  </div>
</div>

<style>
  .cover:focus-visible { outline-offset: -2px; } /* inside the tile, which clips */
  .hov { transition: opacity var(--dur-2) var(--ease-out); }
  .tile:not(:hover, :focus-within) .hov { opacity: 0; }
  .dot { box-shadow: 0 0 0 1px var(--bg-app); }
</style>
