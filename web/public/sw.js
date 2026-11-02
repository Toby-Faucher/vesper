// Vesper's service worker. Web Push only: it shows the notifications the server sends while no Vesper tab is open.
// There is no fetch handler and no cache on purpose: the app always loads fresh from the server.
// It is registered only after someone turns push on in Settings > Notifications, and removed when they turn it off or
// log out.

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

// The payload is { kind, room_id, title, body }, decrypted by the browser. It never holds a message text.
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
  // One notification per room: a newer one replaces the older.
  const tag = room ? `room-${room}` : 'vesper';
  e.waitUntil(self.registration.showNotification(title, { body, tag, icon: '/icon-192.png', data: { room } }));
});

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
