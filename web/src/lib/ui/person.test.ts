import { expect, it } from 'vitest';
import { canModerate, closePerson, person, togglePerson } from './person.svelte';

it('toggle_opens_switches_and_closes_on_the_same_anchor', () => {
  // Not plain objects: $state would proxy those (it does not proxy elements), and the identity check would fail.
  const el = class {};
  const a = new el() as HTMLElement;
  const b = new el() as HTMLElement;
  togglePerson('u1', a, 'top');
  expect([person.id, person.anchor, person.side]).toEqual(['u1', a, 'top']);
  togglePerson('u1', b); // the same person from another place moves the card there
  expect([person.id, person.anchor, person.side]).toEqual(['u1', b, 'right']);
  togglePerson('u1', b);
  expect(person.id).toBeNull();
  togglePerson('u2', a);
  closePerson();
  expect([person.id, person.anchor]).toEqual([null, null]);
});

it('moderator_actions_follow_the_server_rank_rule', () => {
  const u = (user_id: string, role: 'member' | 'moderator' | 'owner') => ({ user_id, role });
  const [member, member2, mod, mod2, owner] = [u('m', 'member'), u('m2', 'member'), u('d', 'moderator'), u('d2', 'moderator'), u('o', 'owner')];
  // A member sees none, on anyone.
  for (const t of [member2, mod, owner]) expect(canModerate(member, t)).toBe(false);
  // A moderator acts on members only.
  expect(canModerate(mod, member)).toBe(true);
  expect(canModerate(mod, mod2)).toBe(false);
  expect(canModerate(mod, owner)).toBe(false);
  // The owner acts on anyone else.
  expect(canModerate(owner, member)).toBe(true);
  expect(canModerate(owner, mod)).toBe(true);
  // Nobody on themselves, and nothing while either side is unknown.
  for (const me of [member, mod, owner]) expect(canModerate(me, me)).toBe(false);
  expect(canModerate(undefined, member)).toBe(false);
  expect(canModerate(owner, undefined)).toBe(false);
});
