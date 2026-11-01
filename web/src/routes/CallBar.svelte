<script lang="ts">
  import * as I from '../lib/ui/icons';
  import type { Peer } from '../lib/gen/Peer';
  import { inRoom } from '../lib/peers';
  import { href } from '../lib/route';
  import { session } from '../lib/session.svelte';
  import { dismissOffered, leave, toggleDeafen, toggleMute, useOfferedMic, voice } from '../lib/voice.svelte';
  import { enter, exit } from '../lib/ui/motion';
  import { togglePerson } from '../lib/ui/person.svelte';
  import { qualityText } from '../lib/ui/quality';
  import { keyName } from '../lib/ui/keys';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import IconButton from '../lib/ui/c/IconButton.svelte';
  import Status from '../lib/ui/c/Status.svelte';
  // The call, floating at the bottom of the main pane while you are in voice. Everything here is read from the voice
  // store; nothing animates while the call runs (the speaking ring is an instant toggle from the store's 100 ms poll).
  let { height = $bindable(0) }: { height?: number } = $props();

  // Enter on mount. On leave the bar goes at once (no stale buttons or notices) and an inert copy of it fades out.
  function motion(n: HTMLElement) {
    enter(n.firstElementChild!, 'rise');
    const host = n.parentElement;
    return () => {
      const ghost = n.cloneNode(true) as HTMLElement;
      ghost.inert = true;
      ghost.setAttribute('aria-hidden', 'true');
      host?.append(ghost);
      void exit(ghost.firstElementChild!, 'rise').then(() => ghost.remove());
    };
  }

  const AVATARS = 5; // drawn; the rest is "+n"
  const room = $derived(session.state.rooms.find((r) => r.id === voice.joinedRoom));
  const me = $derived(session.state.me?.user);
  const people = $derived.by<Peer[]>(() => {
    const others = voice.joinedRoom === null ? [] : inRoom(session.state.voice, voice.joinedRoom).filter((p) => p.user_id !== me?.user_id);
    return me ? [{ user_id: me.user_id, name: me.display_name, room_id: voice.joinedRoom ?? 0, muted: voice.muted, deafened: voice.deafened, force_muted: voice.forceMuted }, ...others] : others;
  });
  const user = (p: Peer) => session.state.users[p.user_id] ?? { user_id: p.user_id, display_name: p.name, accent: '' };
  const LEVEL = { good: 3, fair: 2, poor: 1, unknown: 0 } as const;
  const qText = $derived(voice.quality === 'good' ? 'Good connection' : voice.quality === 'unknown' ? 'Checking connection' : qualityText(voice.quality, voice.diag));
  const [qWords, qFigure = ''] = $derived(qText.split(': '));
  const key = $derived(keyName(voice.settings.pushToTalk.key));
  const warning = $derived(
    voice.micWarning === 'mic' ? 'No sound from your microphone. Check the input device, its volume, and the mute switch.'
      : voice.micWarning === 'pipeline' ? 'The audio pipeline stopped. Leave and join again.' : '',
  );
  const noise = $derived(`Noise suppression: ${voice.mode}${voice.modeReason ? ` (${voice.modeReason})` : ''}`);
  const notice = $derived(voice.deviceError || (voice.forceMuted ? 'A moderator muted you.' : '') || warning);
</script>

<div class="pointer-events-none absolute inset-x-0 bottom-3 flex justify-center px-4" bind:clientHeight={height} {@attach motion}>
  <section aria-label="Call" class="bar pointer-events-auto flex max-w-full min-w-0 flex-col rounded-lg border border-line bg-raised shadow-float">
    <div class="flex h-13 min-w-0 items-center gap-1 pr-2 pl-3">
      <!-- A fixed width, so the bar does not move when the quality text changes. -->
      <div class="mr-2 flex w-48 min-w-0 flex-col">
        <a class="truncate text-sm font-medium text-fg hover:underline" href={href({ name: 'room', id: voice.joinedRoom ?? 0 })} title={room?.name}>{room?.name}</a>
        <!-- Only the words are live; the figure changes every 2 s poll and would be read out each time. -->
        <span class="flex min-w-0 items-center gap-1.5 text-xs" class:text-fg-3={voice.quality !== 'poor'} class:text-warn={voice.quality === 'poor'} title={qText}>
          <svg class="shrink-0" width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden="true">
            {#each [1, 2, 3] as n (n)}<rect x={n * 4 - 4} y={10 - n * 3} width="3" height={n * 3 + 1} rx="0.5" opacity={n <= LEVEL[voice.quality] ? 1 : 0.3} />{/each}
          </svg>
          <span class="truncate"><span role="status">{qWords}</span>{#if qFigure}<span>: {qFigure}</span>{/if}</span>
        </span>
      </div>

      <!-- No aria-pressed: the label swap (Mute, Unmute) carries the state (plan Rulings). Tooltips open upwards: below the
           bar there is only the screen edge. -->
      <IconButton icon={voice.muted ? I.micOff : I.mic} label={voice.muted ? 'Unmute' : 'Mute'} tip="up" danger={voice.muted} disabled={voice.forceMuted} onclick={toggleMute} />
      <IconButton icon={voice.deafened ? I.headphonesOff : I.headphones} label={voice.deafened ? 'Undeafen' : 'Deafen'} tip="up" danger={voice.deafened} onclick={toggleDeafen} />
      {#if voice.settings.pushToTalk.enabled}
        <!-- Both texts share one grid cell, so the pill keeps its width while the key goes down and up. Not a live region:
             it changes on every press, and the Mute state already speaks. -->
        <span class="ptt ml-1 grid h-7 shrink-0 items-center rounded-sm px-2 text-xs" class:held={voice.pttHeld}>
          <span class="col-start-1 row-start-1 flex items-center justify-center gap-1.5" class:invisible={!voice.pttHeld}><span class="dot" aria-hidden="true"></span>Talking</span>
          <span class="col-start-1 row-start-1" class:invisible={voice.pttHeld}>Push to talk: hold {key}</span>
        </span>
      {/if}

      <span class="mx-2 h-6 w-px shrink-0 bg-subtle" aria-hidden="true"></span>
      <ul class="flex shrink-0 items-center gap-1" aria-label="In this call">
        {#each people.slice(0, AVATARS) as p (p.user_id)}
          {@const s = voice.speaking.has(p.user_id)}
          <li>
            <button type="button" class="who flex size-7 items-center justify-center rounded-full" title={p.name}
              onclick={(e) => togglePerson(p.user_id, e.currentTarget, 'top')}>
              <Avatar user={user(p)} size={20} speaking={s} />
              <span class="sr-only">{p.name}{s ? ', speaking' : ''}{p.deafened ? ', deafened' : p.muted || p.force_muted ? ', muted' : ''}</span>
            </button>
          </li>
        {/each}
        {#if people.length > AVATARS}<li class="px-1 text-xs text-fg-2 tabular-nums">+{people.length - AVATARS}</li>{/if}
      </ul>
      <span class="mx-2 h-6 w-px shrink-0 bg-subtle" aria-hidden="true"></span>

      <a class="details flex h-8 shrink-0 items-center rounded-md px-2 text-sm text-fg-2" href={href({ name: 'settings', section: 'diagnostics' })}>Call details</a>
      <Button variant="danger-ghost" onclick={leave}>Leave</Button>
    </div>

    <!-- One status line, always the same height, so the bar does not jump when a notice comes and goes. The noise
         suppression mode stays at its end, so a force-muted user still sees it. -->
    <div class="line flex h-7 min-w-0 items-center gap-2 border-t border-subtle px-3 text-xs text-fg-3">
      {#if voice.error}
        <p class="min-w-0 truncate text-danger" role="alert" title={voice.error}>{voice.error}</p>
      {:else if notice}
        <Status kind="warn"><span class="block truncate" title={notice}>{notice}</span></Status>
      {:else if voice.offered}
        <p class="min-w-0 truncate text-fg-2" role="status">New microphone: {voice.offered.label || voice.offered.deviceId}</p>
        <button type="button" class="link" onclick={useOfferedMic}>Use it</button>
        <button type="button" class="link" onclick={dismissOffered}>Dismiss</button>
      {/if}
      <p class="noise min-w-0 truncate" role="status" title={noise}>{noise}</p>
    </div>
  </section>
</div>

<style>
  /* The speaking ring's gap is drawn in the sidebar colour; on the bar it takes the bar's own. */
  .bar { --bg-sidebar: var(--bg-raised); }
  .ptt { background: var(--bg-element); border: 1px solid var(--line-subtle); color: var(--fg-2); }
  .ptt.held { color: var(--fg); }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--accent); }
  .who { transition: background-color var(--dur-1) var(--ease-out); }
  .who:hover, .details:hover { background: var(--bg-hover); }
  .details { transition: background-color var(--dur-1) var(--ease-out), color var(--dur-1) var(--ease-out); }
  .details:hover { color: var(--fg); }
  .line :global(.status.warn) { min-width: 0; margin: 0 -8px; padding: 2px 8px; font-size: 12px; line-height: 16px; }
  .line :global(.status .mark) { height: 16px; }
  /* The mode always sits on the right, so it does not jump when a notice comes; next to one it gives up its width first. */
  .noise { margin-left: auto; }
  .noise:not(:first-child) { flex-shrink: 4; }
  .link { flex-shrink: 0; color: var(--fg-2); text-decoration: underline; text-underline-offset: 2px; }
  .link:hover { color: var(--fg); }
</style>
