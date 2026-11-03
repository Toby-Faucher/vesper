<script lang="ts">
  import { onMount } from 'svelte';
  import { ApiFailure, errorText } from '../../lib/api';
  import type { PushDevice } from '../../lib/gen/PushDevice';
  import { STATE_TEXT, loadPush, removeDevice, sendTest, setKinds, turnOff, turnOn, type PushView } from '../../lib/push';
  import Button from '../../lib/ui/c/Button.svelte';
  import Switch from '../../lib/ui/c/Switch.svelte';
  // Settings > Notifications > While Vesper is closed: Web Push for mentions and "started voice", per device. The browser
  // asks for permission only from the switch.
  let view = $state<PushView | null>(null);
  let busy = $state(false);
  let error = $state('');
  let tested = $state('');
  const on = $derived(view?.state === 'on');
  const canTurn = $derived(view?.state === 'on' || view?.state === 'off' || view?.state === 'renew');

  async function refresh() {
    try {
      view = await loadPush();
      error = '';
    } catch (e) {
      error = errorText(e);
    }
  }

  async function toggle(e: Event & { currentTarget: HTMLInputElement }) {
    const box = e.currentTarget;
    busy = true;
    error = tested = '';
    try {
      if (box.checked && view?.publicKey) await turnOn(view.publicKey);
      else if (!box.checked) await turnOff();
    } catch (err) {
      error = err instanceof ApiFailure ? err.detail || errorText(err) : 'The browser could not turn this on. Try again.';
    } finally {
      await refresh();
      busy = false;
      box.checked = on;
    }
  }

  async function kinds(change: Partial<Pick<PushDevice, 'mentions' | 'voice' | 'dms'>>) {
    if (!view?.mine) return;
    try {
      await setKinds({ mentions: view.mine.mentions, voice: view.mine.voice, dms: view.mine.dms, ...change });
    } catch (err) {
      error = errorText(err);
    }
    await refresh();
  }

  async function test() {
    tested = error = '';
    try {
      await sendTest();
      tested = 'Sent. It shows when no Vesper tab is in front; with this tab open, look for it in your system tray.';
    } catch (err) {
      error = err instanceof ApiFailure && err.detail ? err.detail : errorText(err);
    }
  }

  async function remove(d: PushDevice) {
    try {
      await removeDevice(d.id);
    } catch (err) {
      error = errorText(err);
    }
    await refresh();
  }

  onMount(() => void refresh());
</script>

<svelte:window onfocus={() => void refresh()} />

<section role="group" aria-labelledby="notify-push">
  <div>
    <div class="flex h-8 items-center"><h3 id="notify-push" class="text-base font-medium text-fg">While Vesper is closed</h3></div>
    <p class="text-sm text-fg-2">
      Mentions, direct messages, and someone starting voice in an empty room, when no Vesper tab is open. The notification goes through your
      browser's push service (Google, Mozilla, Apple or Microsoft), encrypted: it never holds what was written. On iPhone and iPad it works only from the Home Screen.
    </p>
  </div>
  <div>
    <Switch label="Notify this device while Vesper is closed" checked={on} disabled={!canTurn || busy || !view}
      aria-describedby="push-state" onchange={toggle} />
    <p id="push-state" role="status" class="-mt-1 pr-11 text-xs text-fg-2">{view ? STATE_TEXT[view.state] : 'Checking this browser.'}</p>
  </div>
  {#if on && view?.mine}
    <ul class="kinds flex flex-col gap-3">
      <li><Switch label="Mentions" checked={view.mine.mentions} onchange={(e) => kinds({ mentions: e.currentTarget.checked })} /></li>
      <li><Switch label="Direct messages" checked={view.mine.dms} onchange={(e) => kinds({ dms: e.currentTarget.checked })} /></li>
      <li><Switch label="Someone starts voice" help="At most once every 10 minutes per room." checked={view.mine.voice}
        onchange={(e) => kinds({ voice: e.currentTarget.checked })} /></li>
    </ul>
    <div><Button size="sm" onclick={test}>Send a test notification</Button></div>
    {#if tested}<p role="status" class="text-xs text-fg-2">{tested}</p>{/if}
  {/if}
  {#if error}<p role="alert" class="text-sm text-danger">{error}</p>{/if}
  {#if view?.devices.length}
    <ul aria-label="Devices that get notifications" class="devices">
      {#each view.devices as d (d.id)}
        <li class="flex items-center gap-4 px-3 py-2.5">
          <p class="flex min-w-0 flex-1 items-center gap-2 text-sm text-fg" id="push-device-{d.id}">
            <span class="truncate">{d.label}</span>
            {#if d.current}<span class="shrink-0 rounded-sm bg-selected px-1.5 text-xs text-fg-2">This device</span>{/if}
          </p>
          <Button size="sm" variant="ghost" aria-describedby="push-device-{d.id}" onclick={() => remove(d)}>Remove</Button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  section { display: flex; flex-direction: column; gap: 16px; }
  .kinds { padding-left: 12px; border-left: 1px solid var(--line-subtle); }
  .devices { border: 1px solid var(--line-subtle); border-radius: 6px; }
  .devices li + li { border-top: 1px solid var(--line-subtle); }
</style>
