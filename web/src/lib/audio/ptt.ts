export type PttEvent = {
  type: 'keydown' | 'keyup';
  code: string;
  pttKey: string;
  enabled: boolean;
  /** Target is an input/textarea/select: the user is typing, not talking. */
  typing: boolean;
  /** Settings is waiting for the user to pick a new PTT key. */
  capturing: boolean;
};

/** Held-state reducer. Release is never ignored (else a swallowed keyup leaves the mic hot); only press is filtered. */
export function pttKeyEvent(held: boolean, e: PttEvent): { held: boolean; prevent: boolean } {
  if (e.capturing || e.code !== e.pttKey) return { held, prevent: false };
  if (e.type === 'keyup') return { held: false, prevent: false };
  if (!e.enabled || e.typing) return { held, prevent: false };
  return { held: true, prevent: true }; // prevent: Space would click a focused button
}
