<script lang="ts">
  import type { Me } from '../lib/gen/Me';
  import Button from '../lib/ui/c/Button.svelte';
  import { passkeyFailure } from './settings/passkeys';
  // "Sign in with a passkey" on the login page. A lazy chunk, loaded only when the browser has WebAuthn, so the login page
  // does not carry the passkey code. `attempt` is the login page's own, so a passkey sign in ends exactly like a password one.
  let { busy, attempt }: { busy: boolean; attempt: (signIn: () => Promise<Me>, fail: (err: unknown) => string) => Promise<void> } = $props();
  const signIn = async () => (await import('../lib/passkey')).signInWithPasskey();
</script>

<div class="mt-3 flex flex-col">
  <Button disabled={busy} onclick={() => attempt(signIn, (err) => passkeyFailure(err, true))}>Sign in with a passkey</Button>
</div>
