import { expect, it } from 'vitest';
import { closePerson, person, togglePerson } from './person.svelte';

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
