import { api } from './api';
import type { PushDevice } from './gen/PushDevice';
import type { PushInfo } from './gen/PushInfo';
import type { PushSubscribe } from './gen/PushSubscribe';

// Web Push for Settings > Notifications (loaded with Settings only). The service worker (public/sw.js) is registered when
// someone turns push on, never before, and the browser asks for permission from that click.

/** What the push switch can be on this device. */
export type PushState =
  | 'unsupported' // no service worker or Push API
  | 'install' // iPhone or iPad in Safari: push needs Vesper on the Home Screen
  | 'server-off' // the server has no VAPID subject
  | 'blocked' // notifications blocked in the browser's site settings
  | 'off'
  | 'on';

export interface PushEnv {
  supported: boolean;
  ios: boolean;
  standalone: boolean;
  publicKey: string | null;
  permission: NotificationPermission;
  /** The browser has a subscription and the server has it as this session's device. */
  subscribed: boolean;
}

export function pushState(e: PushEnv): PushState {
  if (!e.supported) return e.ios && !e.standalone ? 'install' : 'unsupported';
  if (!e.publicKey) return 'server-off';
  if (e.permission === 'denied') return 'blocked';
  return e.subscribed && e.permission === 'granted' ? 'on' : 'off';
}

export const STATE_TEXT: Record<PushState, string> = {
  unsupported: 'This browser cannot get notifications while Vesper is closed.',
  install: 'On iPhone and iPad this works only from the Home Screen: tap Share, then Add to Home Screen, and open Vesper from there.',
  'server-off': 'This server is not set up for notifications while Vesper is closed.',
  blocked: 'Blocked by your browser. Allow notifications for this site in the address bar, then turn this on again.',
  off: 'Off on this device.',
  on: 'On for this device.',
};

export const supported = () => 'serviceWorker' in navigator && typeof PushManager !== 'undefined' && typeof Notification !== 'undefined';
const ios = () => /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
const standalone = () => matchMedia('(display-mode: standalone)').matches;

/** base64url to bytes, for `applicationServerKey`. */
export function keyBytes(b64: string): Uint8Array<ArrayBuffer> {
  const bin = atob(b64.replace(/-/g, '+').replace(/_/g, '/'));
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
}

export function sameBytes(a: ArrayBuffer | null, b: Uint8Array): boolean {
  const x = a ? new Uint8Array(a) : new Uint8Array();
  return x.length === b.length && x.every((v, i) => v === b[i]);
}

/** The body for PUT /api/push/subscription. */
export function subscribeBody(sub: PushSubscription, kinds: { mentions: boolean; voice: boolean }): PushSubscribe {
  const { endpoint = '', keys = {} } = sub.toJSON();
  return { endpoint, p256dh: keys.p256dh ?? '', auth: keys.auth ?? '', ...kinds };
}

const registration = () => navigator.serviceWorker.getRegistration('/');

export interface PushView {
  state: PushState;
  devices: PushDevice[];
  /** This device's row on the server, when push is on here. */
  mine: PushDevice | null;
  publicKey: string | null;
}

/** Reads the server's view and the browser's, and works out the state. */
export async function loadPush(): Promise<PushView> {
  const info = supported() ? await api<PushInfo>('GET', '/api/push') : { public_key: null, devices: [] };
  const sub = supported() ? await registration().then((r) => r?.pushManager.getSubscription() ?? null) : null;
  const mine = info.devices.find((d) => d.current) ?? null;
  const state = pushState({
    supported: supported(), ios: ios(), standalone: standalone(), publicKey: info.public_key,
    permission: supported() ? Notification.permission : 'default', subscribed: !!sub && !!mine,
  });
  return { state, devices: info.devices, mine, publicKey: info.public_key };
}

/** Only from a click: the permission prompt must come first, before any other await. */
export async function turnOn(publicKey: string, kinds = { mentions: true, voice: true }): Promise<NotificationPermission> {
  const p = await Notification.requestPermission();
  if (p !== 'granted') return p;
  await navigator.serviceWorker.register('/sw.js', { scope: '/' });
  const reg = await navigator.serviceWorker.ready;
  const key = keyBytes(publicKey);
  let sub = await reg.pushManager.getSubscription();
  // One made for another server key (a restored database) can never be used again.
  if (sub && !sameBytes(sub.options.applicationServerKey, key)) sub = await sub.unsubscribe().then(() => null);
  sub ??= await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key });
  await api('PUT', '/api/push/subscription', subscribeBody(sub, kinds));
  return p;
}

/** This device's switches. */
export async function setKinds(kinds: { mentions: boolean; voice: boolean }): Promise<void> {
  const sub = await registration().then((r) => r?.pushManager.getSubscription() ?? null);
  if (sub) await api('PUT', '/api/push/subscription', subscribeBody(sub, kinds));
}

/** Push off on this device: the server forgets it, the browser drops the subscription and the worker. */
export async function turnOff(): Promise<void> {
  await api('DELETE', '/api/push/subscription');
  const reg = await registration();
  await reg?.pushManager.getSubscription().then((s) => s?.unsubscribe());
  await reg?.unregister();
}

export const sendTest = () => api('POST', '/api/push/test');
export const removeDevice = (id: number) => api('DELETE', `/api/push/devices/${id}`);
