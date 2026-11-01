<script lang="ts">
  import Button from '../../lib/ui/c/Button.svelte';
  import Switch from '../../lib/ui/c/Switch.svelte';
  import { loadPrefs, permission, requestMentions, savePrefs, testNotification } from '../../lib/ui/notify';
  // Settings > Notifications: mention notifications (the browser asks only when the switch is turned on) and the join and
  // leave sounds. Both off until turned on; saved to vesper.notify.
  let prefs = $state(loadPrefs());
  let perm = $state(permission());
  const on = $derived(prefs.mentions && perm === 'granted');
  const STATE: Record<typeof perm, string> = {
    granted: 'Allowed',
    denied: 'Blocked by your browser. Allow notifications for this site in the address bar, then turn this on again.',
    default: 'Not asked yet',
    unsupported: 'This browser does not show notifications.',
  };

  async function setMentions(e: Event & { currentTarget: HTMLInputElement }) {
    const box = e.currentTarget;
    if (box.checked) perm = await requestMentions(); // turns mentions on when allowed
    else savePrefs({ ...loadPrefs(), mentions: false });
    prefs = loadPrefs();
    box.checked = on; // a refusal leaves the switch off
  }
  function setSounds(e: Event & { currentTarget: HTMLInputElement }) {
    savePrefs({ ...loadPrefs(), sounds: e.currentTarget.checked });
    prefs = loadPrefs();
  }
</script>

<!-- Permission can change in the address bar while this page is open. -->
<svelte:window onfocus={() => (perm = permission())} />

<div class="flex flex-col gap-3">
  <div>
    <Switch label="Notify me when someone mentions me" checked={on} disabled={perm === 'unsupported'}
      aria-describedby="mention-state" onchange={setMentions} />
    <p id="mention-state" role="status" class="-mt-1 pr-11 text-xs text-fg-2">{STATE[perm]}</p>
    {#if perm === 'granted'}
      <div class="mt-2"><Button size="sm" onclick={testNotification}>Send a test notification</Button></div>
    {/if}
  </div>
  <Switch label="Play a sound when someone joins or leaves your call" checked={prefs.sounds} onchange={setSounds} />
</div>
