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
      return second && /^[1-9]\d{0,15}$/.test(second) ? { name: 'room', id: Number(second) } : { name: 'home' };
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
