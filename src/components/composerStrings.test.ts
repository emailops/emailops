// The composers' placeholders and tooltips are user-visible strings, so they
// go through i18n like the rest of the UI. The JSX-literal guard only catches
// text children, not attribute values, which is how "Add recipients..." stayed
// English in every locale.

import { describe, expect, it } from 'vitest';
import composeModal from './ComposeModal.tsx?raw';
import composeTabView from './EmailView/ComposeTabView.tsx?raw';
import replyCompose from './EmailView/ReplyCompose.tsx?raw';

const COMPOSERS: [string, string][] = [
  ['ComposeModal', composeModal],
  ['ComposeTabView', composeTabView],
  ['ReplyCompose', replyCompose],
];

/** `placeholder=` / `title=` attribute values. */
const ATTR = /\b(?:placeholder|title)=(\{[^}]*\}|"[^"]*")/g;
/** A translation call — the one allowed source of text. */
const T_CALL = /\bt\('[^']*'(?:,[^)]*)?\)/g;
/** Quoted text with at least two letters left after the t() calls are removed. */
const TEXT = /['"][^'"]*[A-Za-z]{2}[^'"]*['"]/;

function literalAttributes(source: string): string[] {
  return [...source.matchAll(ATTR)].map((m) => m[0]).filter((attr) => TEXT.test(attr.replace(T_CALL, '')));
}

describe('composer strings', () => {
  it.each(COMPOSERS)('%s has no hard-coded placeholder or title text', (_name, source) => {
    expect(literalAttributes(source)).toEqual([]);
  });
});
