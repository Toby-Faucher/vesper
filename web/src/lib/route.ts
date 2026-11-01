// Paths and the screens they open. App.svelte renders from parse(route.path).
export const SETTINGS_SECTIONS = ['voice', 'notifications', 'appearance', 'account', 'diagnostics'] as const;
export const ADMIN_SECTIONS = ['invites', 'members', 'rooms', 'disk'] as const;
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];
export type AdminSection = (typeof ADMIN_SECTIONS)[number];

export type Route =
  | { name: 'room'; id: number }
  | { name: 'settings'; section: SettingsSection }
  | { name: 'admin'; section: AdminSection }
  | { name: 'welcome' }
  | { name: 'login' }
  | { name: 'invite' }
  | { name: 'home' }
  | { name: 'gallery' }
  | { name: 'notfound' };

const pick = <T extends string>(list: readonly T[], s: string | undefined): T => (list.includes(s as T) ? (s as T) : list[0]);

/** `/` and a room path without a valid id give home; an unknown section gives the first one; any other path is not found. */
export function parse(path: string): Route {
  const [first = '', second, ...rest] = path.replace(/\/+$/, '').split('/').slice(1);
  if (rest.length) return { name: 'notfound' };
  switch (first) {
    case '':
      return { name: 'home' };
    case 'r':
      return second && /^[1-9]\d{0,14}$/.test(second) ? { name: 'room', id: Number(second) } : { name: 'home' };
    case 'settings':
      return { name: 'settings', section: pick(SETTINGS_SECTIONS, second) };
    case 'admin':
      return { name: 'admin', section: pick(ADMIN_SECTIONS, second) };
  }
  if (second !== undefined) return { name: 'notfound' };
  if (first === 'welcome' || first === 'login' || first === 'invite') return { name: first };
  if (first === '_gallery') return { name: 'gallery' };
  return { name: 'notfound' };
}

export function href(r: Route): string {
  switch (r.name) {
    case 'room':
      return `/r/${r.id}`;
    case 'settings':
    case 'admin':
      return `/${r.name}/${r.section}`;
    case 'gallery':
      return '/_gallery';
    case 'home':
    case 'notfound':
      return '/';
    default:
      return `/${r.name}`;
  }
}

const OUTSIDE = new Set<Route['name']>(['login', 'invite', 'gallery', 'notfound']);

/** Where sign in returns to: `?next=` with the path, unless that is home. */
export function loginHref(from: string): string {
  const next = safeNext(from);
  return next && next !== '/' ? `/login?next=${encodeURIComponent(next)}` : '/login';
}

/**
 * The `next` of /login as an app path, or null. Only a path that starts with one `/` and parses to a page inside the app:
 * never `//host` or `/\host` (browsers read both as another site), a backslash, a control character (the URL parser drops
 * tabs and newlines, which can turn `/\t/host` into `//host`) or a scheme. The result is rebuilt from the parsed route.
 */
export function safeNext(raw: string | null): string | null {
  if (!raw || raw[0] !== '/' || raw[1] === '/' || raw.includes('\\') || /[\x00-\x1f\x7f]/.test(raw) || raw.includes(':')) return null;
  const r = parse(raw.split(/[?#]/)[0]);
  return OUTSIDE.has(r.name) ? null : href(r);
}
