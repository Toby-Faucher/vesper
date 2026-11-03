import { api } from './api';
import { fromBase64Url, toBase64Url } from './base64url';
import type { Me } from './gen/Me';
import type { PasskeyChallenge } from './gen/PasskeyChallenge';
import type { PasskeyInfo } from './gen/PasskeyInfo';

// Passkeys (WebAuthn). The server sends options as JSON with base64url byte fields; the browser API wants buffers. This file
// converts both ways and runs the two ceremonies. It is loaded only when a passkey is used, not with the login page.

type Json = Record<string, unknown>;
type IdEntry = Json & { id: string };

export const passkeysSupported = (): boolean => typeof window !== 'undefined' && !!window.PublicKeyCredential;

const withIds = (list: unknown): unknown => (Array.isArray(list) ? (list as IdEntry[]).map((c) => ({ ...c, id: fromBase64Url(c.id) })) : list);

/** Server options for `navigator.credentials.create`: the byte fields decoded. */
export function creationOptions(options: unknown): PublicKeyCredentialCreationOptions {
  const pk = (options as { publicKey: Json & { user: Json & { id: string }; challenge: string } }).publicKey;
  return { ...pk, challenge: fromBase64Url(pk.challenge), user: { ...pk.user, id: fromBase64Url(pk.user.id) }, excludeCredentials: withIds(pk.excludeCredentials) } as unknown as PublicKeyCredentialCreationOptions;
}

/** Server options for `navigator.credentials.get`. The `mediation` the server adds is dropped: the button opens the browser's own dialog. */
export function requestOptions(options: unknown): PublicKeyCredentialRequestOptions {
  const pk = (options as { publicKey: Json & { challenge: string } }).publicKey;
  return { ...pk, challenge: fromBase64Url(pk.challenge), allowCredentials: withIds(pk.allowCredentials) } as unknown as PublicKeyCredentialRequestOptions;
}

/** The JSON the server reads. Transports and client extension results are left out: the server needs neither. */
export function attestationJson(c: { id: string; rawId: ArrayBuffer; response: { attestationObject: ArrayBuffer; clientDataJSON: ArrayBuffer } }): Json {
  const r = c.response;
  return { id: c.id, rawId: toBase64Url(c.rawId), type: 'public-key', response: { attestationObject: toBase64Url(r.attestationObject), clientDataJSON: toBase64Url(r.clientDataJSON) } };
}

export function assertionJson(c: {
  id: string;
  rawId: ArrayBuffer;
  response: { authenticatorData: ArrayBuffer; clientDataJSON: ArrayBuffer; signature: ArrayBuffer; userHandle: ArrayBuffer | null };
}): Json {
  const r = c.response;
  return {
    id: c.id,
    rawId: toBase64Url(c.rawId),
    type: 'public-key',
    response: {
      authenticatorData: toBase64Url(r.authenticatorData),
      clientDataJSON: toBase64Url(r.clientDataJSON),
      signature: toBase64Url(r.signature),
      userHandle: r.userHandle ? toBase64Url(r.userHandle) : null,
    },
  };
}

/** Adds a passkey to the signed-in account. Rejects with the browser's error (a cancel is a NotAllowedError) or an ApiFailure. */
export async function addPasskey(name: string): Promise<PasskeyInfo> {
  const start = await api<PasskeyChallenge>('POST', '/api/passkeys/register/start');
  const cred = (await navigator.credentials.create({ publicKey: creationOptions(start.options) })) as PublicKeyCredential | null;
  if (!cred) throw new Error('no credential');
  return api<PasskeyInfo>('POST', '/api/passkeys/register/finish', {
    challenge_id: start.challenge_id,
    credential: attestationJson(cred as unknown as Parameters<typeof attestationJson>[0]),
    name: name.trim() || undefined,
  });
}

/** Signs in with a passkey; the server sets the same session cookie as a password sign in. */
export async function signInWithPasskey(): Promise<Me> {
  const start = await api<PasskeyChallenge>('POST', '/api/passkeys/login/start');
  const cred = (await navigator.credentials.get({ publicKey: requestOptions(start.options) })) as PublicKeyCredential | null;
  if (!cred) throw new Error('no credential');
  return api<Me>('POST', '/api/passkeys/login/finish', {
    challenge_id: start.challenge_id,
    credential: assertionJson(cred as unknown as Parameters<typeof assertionJson>[0]),
  });
}
