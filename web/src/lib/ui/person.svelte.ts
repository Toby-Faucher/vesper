// The one open person card: whose, and the button it hangs from. The sidebar and the call bar open it; the shell draws it.
export const person = $state({ id: null as string | null, anchor: null as HTMLElement | null, side: 'right' as 'right' | 'top' });

/** Opens the card for `id` under `anchor`, or closes it when that card is already open there. */
export function togglePerson(id: string, anchor: HTMLElement, side: 'right' | 'top' = 'right'): void {
  const same = person.id === id && person.anchor === anchor;
  person.id = same ? null : id;
  person.anchor = same ? null : anchor;
  person.side = side;
}

export function closePerson(): void {
  person.id = null;
  person.anchor = null;
}
