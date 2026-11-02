import { afterEach, describe, expect, it, vi } from 'vitest';
import { keyBytes, pushState, sameBytes, subscribeBody, turnOn, turnOff, type PushEnv } from './push';

const env = (o: Partial<PushEnv> = {}): PushEnv => ({
  supported: true, ios: false, standalone: false, publicKey: 'k', permission: 'default', subscribed: false, ...o,
});

describe('pushState', () => {
  it('is honest about why push cannot be on', () => {
    expect(pushState(env({ supported: false }))).toBe('unsupported');
    expect(pushState(env({ supported: false, ios: true }))).toBe('install');
    expect(pushState(env({ supported: false, ios: true, standalone: true }))).toBe('unsupported');
    expect(pushState(env({ publicKey: null, permission: 'granted', subscribed: true }))).toBe('server-off');
    expect(pushState(env({ permission: 'denied', subscribed: true }))).toBe('blocked');
  });
  it('is on only with permission and a subscription the server knows', () => {
    expect(pushState(env())).toBe('off');
    expect(pushState(env({ permission: 'granted' }))).toBe('off');
    expect(pushState(env({ subscribed: true }))).toBe('off');
    expect(pushState(env({ permission: 'granted', subscribed: true }))).toBe('on');
  });
});

describe('keys', () => {
  it('decodes base64url and compares bytes', () => {
    expect([...keyBytes('-_8')]).toEqual([0xfb, 0xff]);
    expect(sameBytes(new Uint8Array([1, 2]).buffer, new Uint8Array([1, 2]))).toBe(true);
    expect(sameBytes(new Uint8Array([1, 3]).buffer, new Uint8Array([1, 2]))).toBe(false);
    expect(sameBytes(null, new Uint8Array([1]))).toBe(false);
  });
  it('sends the subscription and the switches', () => {
    const sub = { toJSON: () => ({ endpoint: 'https://e', keys: { p256dh: 'p', auth: 'a' } }) } as unknown as PushSubscription;
    expect(subscribeBody(sub, { mentions: true, voice: false })).toEqual({ endpoint: 'https://e', p256dh: 'p', auth: 'a', mentions: true, voice: false });
  });
});

describe('turnOn and turnOff', () => {
  const calls: string[] = [];
  const sub = {
    options: { applicationServerKey: keyBytes('AQI').buffer },
    toJSON: () => ({ endpoint: 'https://e', keys: { p256dh: 'p', auth: 'a' } }),
    unsubscribe: async () => (calls.push('unsubscribe'), true),
  };
  const reg = {
    pushManager: {
      getSubscription: async () => (calls.push('get'), null as typeof sub | null),
      subscribe: async () => (calls.push('subscribe'), sub),
    },
    unregister: async () => (calls.push('unregister'), true),
  };
  function stub(permission: NotificationPermission) {
    calls.length = 0;
    vi.stubGlobal('Notification', { requestPermission: async () => (calls.push('permission'), permission) });
    vi.stubGlobal('navigator', {
      serviceWorker: {
        register: async () => calls.push('register'),
        ready: Promise.resolve(reg),
        getRegistration: async () => reg,
      },
    });
    vi.stubGlobal('fetch', async (url: string, init: RequestInit) => {
      calls.push(`${init.method} ${url}`);
      return new Response(null, { status: 204 });
    });
  }
  afterEach(() => vi.unstubAllGlobals());

  it('asks first, and does nothing more when refused', async () => {
    stub('denied');
    expect(await turnOn('AQI')).toBe('denied');
    expect(calls).toEqual(['permission']);
  });
  it('registers the worker, subscribes and tells the server', async () => {
    stub('granted');
    expect(await turnOn('AQI')).toBe('granted');
    expect(calls).toEqual(['permission', 'register', 'get', 'subscribe', 'PUT /api/push/subscription']);
  });
  it('drops a subscription made for another server key', async () => {
    stub('granted');
    reg.pushManager.getSubscription = async () => (calls.push('get'), sub);
    await turnOn('AQM');
    expect(calls).toEqual(['permission', 'register', 'get', 'unsubscribe', 'subscribe', 'PUT /api/push/subscription']);
  });
  it('turning off tells the server, then removes the subscription and the worker', async () => {
    stub('granted');
    reg.pushManager.getSubscription = async () => (calls.push('get'), sub);
    await turnOff();
    expect(calls).toEqual(['DELETE /api/push/subscription', 'get', 'unsubscribe', 'unregister']);
  });
});
