// Its own file so the shell's search box (first load) does not pull in the parser (searchq.ts, the search chunk).
/** The longest box the dialog takes (its `maxlength`), and the most words sent: a long paste is cut, not sent whole. */
export const MAX_QUERY = 200;
