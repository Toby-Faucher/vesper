import { expect, it } from 'vitest';
import { menuMove, toolMove } from './menu';

const labels = ['Copy link', 'Edit', 'Pin', 'Delete', 'Copy text'];

it('arrows_wrap', () => {
  expect(menuMove('ArrowDown', 0, labels)).toBe(1);
  expect(menuMove('ArrowDown', 4, labels)).toBe(0);
  expect(menuMove('ArrowUp', 0, labels)).toBe(4);
  // Nothing focused yet: Down goes to the first, Up to the last.
  expect(menuMove('ArrowDown', -1, labels)).toBe(0);
  expect(menuMove('ArrowUp', -1, labels)).toBe(4);
});

it('home_and_end', () => {
  expect(menuMove('Home', 3, labels)).toBe(0);
  expect(menuMove('End', 1, labels)).toBe(4);
});

it('typeahead_by_first_letter_cycles', () => {
  expect(menuMove('d', 0, labels)).toBe(3);
  expect(menuMove('C', 0, labels)).toBe(4);
  expect(menuMove('c', 4, labels)).toBe(0);
  expect(menuMove('x', 0, labels)).toBe(null);
});

it('other_keys_are_left_alone', () => {
  for (const k of [' ', 'Enter', 'Escape', 'Tab', 'ArrowLeft', 'F1']) expect(menuMove(k, 1, labels)).toBe(null);
  expect(menuMove('ArrowDown', -1, [])).toBe(null);
});

it('toolbar_left_right_wrap_home_end', () => {
  expect(toolMove('ArrowRight', 0, 4)).toBe(1);
  expect(toolMove('ArrowRight', 3, 4)).toBe(0);
  expect(toolMove('ArrowLeft', 0, 4)).toBe(3);
  expect(toolMove('Home', 2, 4)).toBe(0);
  expect(toolMove('End', 0, 4)).toBe(3);
  for (const k of ['ArrowUp', 'ArrowDown', 'Enter', ' ', 'Escape', 'Tab', 'a']) expect(toolMove(k, 1, 4)).toBe(null);
});
