<script lang="ts">
  import { onMount } from 'svelte';
  import Avatar from '../lib/ui/c/Avatar.svelte';
  import Button from '../lib/ui/c/Button.svelte';
  import { href } from '../lib/route';
  import { RING_EVERY_MS, ring, stopRing } from '../lib/ring.svelte';
  import { navigate, session } from '../lib/session.svelte';
  import { chime } from '../lib/ui/chime';
  import { enter } from '../lib/ui/motion';
  import { loadPrefs } from '../lib/ui/prefs';
  import { join, leave, voice } from '../lib/voice.svelte';
  // The prompt for an incoming DM call: the caller's picture and name, Accept and Decline. Not modal: you can keep typing
  // until you choose, but focus moves to Accept when it appears and goes back when it ends, and Esc declines. The first
  // ring shows; any others wait behind it. The ring sound repeats while it shows, unless Settings > Notifications has it
  // off. A browser that has not had a click or key on the page yet keeps the sound silent (autoplay policy).
  const current = $derived(ring.list[0]);
  const who = $derived(current ? session.state.users[current.user_id] ?? { user_id: current.user_id, display_name: 'Someone', accent: '' } : null);
  let el = $state<HTMLElement>();
  let accept = $state<HTMLButtonElement | null>(null);
  let opener: HTMLElement | null = null;

  onMount(() => {
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    return () => opener?.isConnected && opener.focus();
  });
  $effect(() => {
    if (!current) return;
    enter(el!, 'rise'); // an 80 ms fade under reduced motion
    accept?.focus();
  });
  // One timer per ring (keyed on the room), so a second caller does not restart the sound.
  $effect(() => {
    if (current === undefined) return;
    const play = () => {
      const p = loadPrefs();
      if (p.ring) chime('ring', voice.settings.speakerId, p.volume);
    };
    play();
    const t = setInterval(play, RING_EVERY_MS);
    return () => clearInterval(t);
  });

  function onAccept() {
    if (!current) return;
    const room = current.room_id;
    stopRing(room);
    if (voice.joined) leave(); // one call at a time
    navigate(href({ name: 'room', id: room }));
    void join(room); // from the click itself: join() opens its audio before its first await
  }
  function onDecline() {
    if (!current) return;
    session.sig?.send({ type: 'decline_ring', room_id: current.room_id });
    stopRing(current.room_id);
  }
</script>

{#if current && who}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_element_to_interactive_role -->
  <div
    bind:this={el}
    role="alertdialog"
    aria-labelledby="ring-title"
    aria-describedby="ring-body"
    class="prompt"
    onkeydown={(e) => e.key === 'Escape' && (e.preventDefault(), onDecline())}
  >
    <span class="pulse"><Avatar user={who} size={40} /></span>
    <div class="min-w-0 flex-1">
      <p id="ring-title" class="truncate text-base font-medium text-fg">{who.display_name}</p>
      <p id="ring-body" class="text-xs text-fg-2">is calling you in a direct message</p>
    </div>
    <Button variant="primary" size="sm" bind:el={accept} onclick={onAccept}>Accept</Button>
    <Button variant="danger" size="sm" onclick={onDecline}>Decline</Button>
  </div>
{/if}

<style>
  .prompt {
    position: fixed;
    right: 16px;
    bottom: 16px;
    z-index: 70;
    display: flex;
    align-items: center;
    gap: 12px;
    width: min(420px, calc(100vw - 32px));
    padding: 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-raised);
    box-shadow: var(--shadow-pop, 0 8px 24px color-mix(in oklab, black, transparent 65%));
  }
  .pulse { display: inline-flex; border-radius: 9999px; }
  /* A soft ring around the picture; not at all under reduced motion. */
  @media (prefers-reduced-motion: no-preference) {
    .pulse { animation: ringing 1.6s var(--ease-out) infinite; }
    @keyframes ringing {
      from { box-shadow: 0 0 0 0 color-mix(in oklab, var(--accent), transparent 40%); }
      to { box-shadow: 0 0 0 10px transparent; }
    }
  }
</style>
