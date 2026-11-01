import { describe, expect, it } from 'vitest';
import { ADMIN_SECTIONS, SETTINGS_SECTIONS, href, loginHref, parse, safeNext, type Route } from './route';

describe('route', () => {
  it('round trips every route', () => {
    const all: Route[] = [
      { name: 'room', id: 1 }, { name: 'room', id: 42 }, { name: 'welcome' }, { name: 'login' }, { name: 'invite' }, { name: 'home' },
      { name: 'gallery' },
      ...SETTINGS_SECTIONS.map((section) => ({ name: 'settings', section }) as const),
      ...ADMIN_SECTIONS.map((section) => ({ name: 'admin', section }) as const),
    ];
    for (const r of all) expect(parse(href(r))).toEqual(r);
  });
  it('a room path without a valid id gives home', () => {
    for (const p of ['/r/abc', '/r/', '/r', '/r/0', '/r/1.5', '/r/-3', '/r/1234567890123456']) expect(parse(p)).toEqual({ name: 'home' });
  });
  it('tolerates a trailing slash', () => {
    expect(parse('/r/7/')).toEqual({ name: 'room', id: 7 });
    expect(parse('/settings/account/')).toEqual({ name: 'settings', section: 'account' });
    expect(parse('/welcome/')).toEqual({ name: 'welcome' });
  });
  it('an unknown or missing section gives the first one', () => {
    expect(parse('/settings/nope')).toEqual({ name: 'settings', section: 'voice' });
    expect(parse('/settings')).toEqual({ name: 'settings', section: 'voice' });
    expect(parse('/admin/x')).toEqual({ name: 'admin', section: 'invites' });
  });
  it('anything else is not found', () => {
    for (const p of ['/nope', '/r/1/extra', '/welcome/x', '/api/uploads/x', '/settings/voice/x']) expect(parse(p)).toEqual({ name: 'notfound' });
  });
  it('a room id has at most 15 digits, so it stays an exact number', () => {
    expect(parse('/r/999999999999999')).toEqual({ name: 'room', id: 999999999999999 });
  });
  it('next after sign in: an app path, rebuilt from the route', () => {
    expect(safeNext('/r/12')).toBe('/r/12');
    expect(safeNext('/r/12/')).toBe('/r/12');
    expect(safeNext('/r/12?x=1#y')).toBe('/r/12');
    expect(safeNext('/settings/account')).toBe('/settings/account');
    expect(safeNext('/')).toBe('/');
  });
  it('next rejects other sites, schemes, backslashes and pages outside the app', () => {
    for (const p of [
      null, '', '//evil.example', '//evil.example/r/1', '/\\evil.example', '/r/1\\x', '\\evil.example', 'javascript:alert(1)',
      '/javascript:alert(1)', 'https://evil.example/r/1', '/\t/evil.example', '/\n/evil.example', 'r/1', '/nope', '/login', '/invite',
      '/_gallery',
      // The route parser alone would turn these into a settings page, so each guard above has a case of its own.
      '/settings/\\evil.example', '/settings/\tx', '/settings/javascript:alert(1)', 'evil.example/settings/voice',
    ])
      expect(safeNext(p), String(p)).toBeNull();
  });
  it('the sign in link keeps the page, except home', () => {
    expect(loginHref('/r/12')).toBe('/login?next=%2Fr%2F12');
    expect(loginHref('/')).toBe('/login');
    expect(loginHref('//evil.example')).toBe('/login');
    expect(safeNext(decodeURIComponent(loginHref('/settings/voice').split('next=')[1]))).toBe('/settings/voice');
  });
});
