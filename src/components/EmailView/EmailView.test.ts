// Unit tests for pure EmailView helpers.
//
// Currently exercises `shouldConsumePendingChatDraft`. The previous
// implementation gated consumption on `pendingChatDraft.emailId ===
// latestEmail.id`, which silently dropped the AI body whenever the
// inbound the draft was written for sat behind a later reply in the
// thread — exactly what produced the "draft chip navigates to the
// thread but the reply body is missing" bug report.

import { describe, expect, it } from 'vitest';
import { prependDraftBody, shouldConsumePendingChatDraft } from './EmailView';

describe('shouldConsumePendingChatDraft', () => {
  it('returns false when there is no pending draft', () => {
    expect(shouldConsumePendingChatDraft(null, ['eml-a', 'eml-b'])).toBe(false);
  });

  it('returns false when the thread has not loaded yet', () => {
    // `navigateToEmail` clears `threadEmails` while it fetches. The
    // effect would otherwise fire before the thread mounts and skip,
    // then have nothing to retrigger on once the data arrives. Treating
    // the empty thread as "not ready" guarantees the effect re-runs
    // when `threadEmails` populates.
    expect(shouldConsumePendingChatDraft({ emailId: 'eml-a' }, [])).toBe(false);
  });

  it('returns true when the inbound is the latest message in the thread', () => {
    expect(shouldConsumePendingChatDraft({ emailId: 'eml-b' }, ['eml-a', 'eml-b'])).toBe(true);
  });

  it('returns true when the inbound is an earlier message in the loaded thread', () => {
    // Regression for the failing case: the chat drafted a reply to
    // `eml-a`, but a later message `eml-b` arrived before the user
    // clicked the draft chip. The body must still land in the inline
    // reply pane — the thread membership is what we should gate on,
    // not "is this the most recent message".
    expect(shouldConsumePendingChatDraft({ emailId: 'eml-a' }, ['eml-a', 'eml-b', 'eml-c'])).toBe(true);
  });

  it('returns false when the inbound is not part of the loaded thread', () => {
    expect(shouldConsumePendingChatDraft({ emailId: 'eml-xyz' }, ['eml-a', 'eml-b'])).toBe(false);
  });
});

describe('prependDraftBody', () => {
  // Regression for "reply textbox should be empty" — when the user clicks
  // Reply (or the inline reply opens fresh on thread switch), the body must
  // start at "" rather than a quoted snippet of the previous email. The AI
  // draft flow still prepends on top of whatever is in the box; when that
  // is empty the result must be exactly the draft, with no trailing newline
  // pair that would push the cursor down on first focus.
  it('returns the draft alone when there is no existing text', () => {
    expect(prependDraftBody('Hi there,', '')).toBe('Hi there,');
  });

  it("preserves the user's typed text below the draft", () => {
    expect(prependDraftBody('Suggested reply.', 'my notes')).toBe('Suggested reply.\n\nmy notes');
  });

  it('preserves multi-paragraph draft bodies verbatim', () => {
    expect(prependDraftBody('Para one.\n\nPara two.', '')).toBe('Para one.\n\nPara two.');
  });
});
