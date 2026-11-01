import { expect, it } from 'vitest';
import { byUsername, canModerate, closePerson, person, profileLabel, togglePerson } from './person.svelte';
import button from './c/PersonButton.svelte?raw';
import item from '../../routes/MessageItem.svelte?raw';
import markdown from '../../routes/Markdown.svelte?raw';
import members from '../../routes/admin/Members.svelte?raw';

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

it('a_mention_finds_its_user_without_case_and_unknown_names_stay_text', () => {
  const users = { a: { user_id: 'a', username: 'Alice' }, b: { user_id: 'b', username: 'bob' } };
  expect(byUsername(users, 'alice')?.user_id).toBe('a');
  expect(byUsername(users, 'BOB')?.user_id).toBe('b');
  expect(byUsername(users, 'carol')).toBeUndefined();
  expect(profileLabel('alice')).toBe('Open profile of alice');
});

it('names_open_the_card_through_the_one_button', () => {
  // The button opens the card from itself (the anchor focus goes back to) and carries the label.
  expect(button).toContain('togglePerson(id, e.currentTarget)');
  expect(button).toContain('aria-label={profileLabel(name)}');
  // A message head: the name is the button, outside the roving toolbar; the avatar clicks it (no second Tab stop).
  const tools = item.slice(item.indexOf('role="toolbar"'));
  expect(item).toMatch(/<PersonButton id=\{m\.author_id\}[^>]*bind:el=\{nameEl\}/);
  expect(tools.slice(0, tools.indexOf('</div>'))).not.toContain('PersonButton');
  expect(item).toContain('onclick={() => nameEl?.click()}');
  // A mention of a known user, and a Members row.
  expect(markdown).toMatch(/\{#if u\}<PersonButton id=\{u\.user_id\}/);
  expect(members).toContain('<PersonButton id={u.user_id}');
});
