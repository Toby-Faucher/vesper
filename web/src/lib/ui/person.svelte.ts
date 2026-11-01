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

const RANK = { member: 0, moderator: 1, owner: 2 } as const;
type Ranked = { role: keyof typeof RANK };
/** The server's voice_target rule: a moderator acts on members, the owner on anyone lower; so never on yourself or an equal. */
export function canModerate(me: Ranked | undefined, them: Ranked | undefined): boolean {
  return !!me && !!them && RANK[me.role] > RANK[them.role];
}

/** Opens the card for `id` as a dialog (from the switcher, where there is no button to hang it from). */
export function openPerson(id: string): void {
  person.id = id;
  person.anchor = null;
}

