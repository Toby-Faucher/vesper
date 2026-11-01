import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiFailure, api, errorText } from './api';

const respond = (status: number, body?: unknown) =>
  vi.fn(async (_p: string, _i: RequestInit) => new Response(body === undefined ? null : JSON.stringify(body), { status }));
afterEach(() => vi.unstubAllGlobals());

describe('api', () => {
  it('api_throws_ApiFailure_with_code', async () => {
    vi.stubGlobal('fetch', respond(429, { code: 'rate_limited', message: 'slow down' }));
    const e = await api('POST', '/api/login', { username: 'a', password: 'test-password-1' }).catch((x) => x);
    expect(e).toBeInstanceOf(ApiFailure);
    expect(e).toMatchObject({ status: 429, code: 'rate_limited', detail: 'slow down' });
    expect(errorText(e)).toBe('Too many tries. Wait a minute and try again.');
  });
  it('sends JSON with same-origin credentials and parses the reply', async () => {
    const f = respond(200, { ok: 1 });
    vi.stubGlobal('fetch', f);
    expect(await api('PATCH', '/api/me', { display_name: 'A' })).toEqual({ ok: 1 });
    const [path, init] = f.mock.calls[0];
    expect(path).toBe('/api/me');
    expect(init).toMatchObject({ method: 'PATCH', credentials: 'same-origin', body: '{"display_name":"A"}' });
    expect(init.headers).toEqual({ 'content-type': 'application/json' });
  });
  it('204 resolves undefined; a body that is not JSON is internal', async () => {
    vi.stubGlobal('fetch', respond(204));
    expect(await api('POST', '/api/logout')).toBeUndefined();
    vi.stubGlobal('fetch', vi.fn(async () => new Response('oops', { status: 502 })));
    await expect(api('GET', '/api/me')).rejects.toMatchObject({ status: 502, code: 'internal' });
  });
  it('a network failure has its own text and never shows a raw code', () => {
    expect(errorText(new TypeError('fetch failed'))).toMatch(/Could not reach/);
    expect(errorText(new ApiFailure(400, 'bad_request'))).not.toMatch(/bad_request/);
  });
});
