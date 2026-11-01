import { describe, expect, it } from 'vitest';
import { ADMIN_SECTIONS, SETTINGS_SECTIONS, href, parse, type Route } from './route';

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
    for (const p of ['/r/abc', '/r/', '/r', '/r/0', '/r/1.5', '/r/-3']) expect(parse(p)).toEqual({ name: 'home' });
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
});
