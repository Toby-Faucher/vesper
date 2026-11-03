// Vesper's service worker. Web Push only: it shows the notifications the server sends while no Vesper tab is open.
// There is no fetch handler and no cache on purpose: the app always loads fresh from the server.
// It is registered only after someone turns push on in Settings > Notifications, and removed when they turn it off or
// log out.

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

// The payload is { kind, room_id, title, body, icon }, decrypted by the browser. It never holds a message text.
// icon is a same-origin path to the sender's picture (DM pushes only).
self.addEventListener('push', (e) => {
  let p = {};
  try {
    p = e.data ? e.data.json() : {};
  } catch {
    // not JSON: the generic text below
  }
  const room = Number.isSafeInteger(p.room_id) && p.room_id > 0 ? p.room_id : 0;
  const title = typeof p.title === 'string' ? p.title.slice(0, 64) : 'Vesper';
  const body = typeof p.body === 'string' ? p.body.slice(0, 200) : 'Something happened in Vesper.';
  // One notification per room and kind: a newer mention replaces an older one, but "started voice" never hides a mention.
  const kind = ['mention', 'dm', 'voice', 'test'].includes(p.kind) ? p.kind : 'other';
  const tag = room ? `${kind}-${room}` : 'vesper';
  // A mention marks the app's badge (no number: the worker does not know the count; the page sets it when it opens).
  // Only where the Badging API exists: Firefox has none.
  const mark = kind === 'mention' || kind === 'dm' ? self.navigator.setAppBadge?.()?.catch(() => {}) : undefined;
  e.waitUntil(Promise.all([picture(p.icon).then((icon) => self.registration.showNotification(title, { body, tag, icon, data: { room } })), mark]));
});

// The sender's picture as a data URL, because a notification icon is fetched without the session cookie. Any failure gives the default icon.
async function picture(path) {
  const fallback = '/icon-192.png';
  if (typeof path !== 'string' || !/^\/api\/users\/[0-9a-f]{32}\/avatar\?v=[\w-]+$/.test(path)) return fallback;
  try {
    const r = await fetch(path, { credentials: 'same-origin' });
    if (!r.ok) return fallback;
    const blob = await r.blob();
    if (blob.size > 512 * 1024) return fallback;
    return await new Promise((ok, no) => {
      const f = new FileReader();
      f.onload = () => ok(f.result);
      f.onerror = no;
      f.readAsDataURL(blob);
    });
  } catch {
    return fallback;
  }
}

// Focus an open Vesper tab and open the room in it (the app navigates in place, so a call goes on), else open a new one.
self.addEventListener('notificationclick', (e) => {
  e.notification.close();
  const room = e.notification.data?.room;
  const url = Number.isSafeInteger(room) && room > 0 ? `/r/${room}` : '/';
  e.waitUntil((async () => {
    const tabs = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    const tab = tabs.find((c) => new URL(c.url).origin === self.location.origin);
    if (tab) {
      await tab.focus();
      tab.postMessage({ open: url });
    } else {
      await self.clients.openWindow(url);
    }
  })());
});

const keyBytes = (b64) => Uint8Array.from(atob(b64.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));

// The push service replaced the subscription: subscribe again with this device's switches, if push is still on here.
self.addEventListener('pushsubscriptionchange', (e) => {
  e.waitUntil((async () => {
    const info = await fetch('/api/push', { credentials: 'same-origin' }).then((r) => (r.ok ? r.json() : null));
    const mine = info?.devices.find((d) => d.current);
    if (!info?.public_key || !mine) return;
    const sub = await self.registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(info.public_key) });
    const { endpoint, keys } = sub.toJSON();
    await fetch('/api/push/subscription', {
      method: 'PUT',
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ endpoint, p256dh: keys.p256dh, auth: keys.auth, mentions: mine.mentions, voice: mine.voice }),
    });
  })());
});
