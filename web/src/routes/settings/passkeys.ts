import { ApiFailure, errorText } from '../../lib/api';

/** Plain text for why adding or using a passkey failed. A cancel or a refusal by the device is a NotAllowedError. */
export function passkeyFailure(err: unknown, signIn = false): string {
  if (err instanceof ApiFailure) {
    if (signIn && err.status === 401) return 'That passkey did not work. Try again, or use your password.';
    if (err.status === 400) return 'That passkey could not be added. Try again.';
    if (err.status === 409) return 'You have the most passkeys allowed. Remove one first.';
    return errorText(err);
  }
  const n = err instanceof DOMException ? err.name : '';
  if (n === 'NotAllowedError' || n === 'AbortError') return 'Cancelled, or your device did not allow it.';
  if (n === 'InvalidStateError') return 'This device already has a passkey for this account.';
  if (n === 'SecurityError') return 'Passkeys need the address you opened Vesper at to match the server setting.';
  return errorText(err);
}
