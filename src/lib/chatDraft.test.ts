import { describe, expect, it } from 'vitest';
import type { ChatDraftContext } from '@/types';
import { draftChipScope, draftContextOf, draftForTurn, planDraftOffer, userTextOf } from './chatDraft';
import { SIGNATURE_ATTR } from './signature';

const signature = `<div ${SIGNATURE_ATTR}=""><p>Ana Lopez</p><p>Example Ltd</p></div>`;

describe('userTextOf', () => {
  it('is the text the user wrote above the signature', () => {
    expect(userTextOf(`<p>The report is attached.</p><p></p>${signature}`)).toBe('The report is attached.');
  });

  it('keeps text the user wrote below the signature', () => {
    expect(userTextOf(`<p>Hi Ana,</p>${signature}<p>PS: see you Thursday.</p>`)).toBe('Hi Ana,\nPS: see you Thursday.');
  });

  it('is empty when the composer holds only the signature', () => {
    expect(userTextOf(`<p></p>${signature}`)).toBe('');
  });

  it('is empty for an empty body', () => {
    expect(userTextOf('')).toBe('');
  });

  it('is the whole body when there is no signature node, as in a draft reopened from the provider', () => {
    expect(userTextOf('<p>Thursday works.</p><p>-- </p><p>Ana Lopez</p>')).toBe('Thursday works.\n-- \nAna Lopez');
  });
});

describe('draftContextOf', () => {
  const composer = {
    kind: 'reply' as const,
    subject: 'Re: Q3 report',
    to: ['ana@example.com'],
    cc: [],
    bodyHtml: `<p>The report is attached.</p>${signature}`,
    replyToEmailId: 'email-1',
  };

  it('carries the user text, the subject, the recipients and the email it answers', () => {
    expect(draftContextOf(composer)).toEqual({
      kind: 'reply',
      subject: 'Re: Q3 report',
      to: ['ana@example.com'],
      cc: [],
      body: 'The report is attached.',
      replyToEmailId: 'email-1',
    });
  });

  it('is null while the composer holds only the signature', () => {
    expect(draftContextOf({ ...composer, bodyHtml: `<p></p>${signature}` })).toBeNull();
  });

  it('names no email for a new message', () => {
    expect(draftContextOf({ ...composer, kind: 'new', replyToEmailId: null })?.replyToEmailId).toBeNull();
  });
});

describe('draftForTurn', () => {
  const snapshot: ChatDraftContext = {
    kind: 'reply',
    subject: 'Re: Q3 report',
    to: ['ana@example.com'],
    cc: [],
    body: 'The report is attached.',
    replyToEmailId: 'email-1',
  };
  const base = { snapshot, accountId: 'acc-1', chatAccountId: 'acc-1', disabled: false, research: false };

  it('sends the draft with its thread when the composer is on the chat account', () => {
    expect(draftForTurn(base)).toEqual(snapshot);
  });

  it('sends nothing while the user has the draft chip switched off', () => {
    expect(draftForTurn({ ...base, disabled: true })).toBeNull();
  });

  it('sends nothing on a research turn', () => {
    expect(draftForTurn({ ...base, research: true })).toBeNull();
  });

  it('sends the draft without its thread when the composer is on another account', () => {
    expect(draftForTurn({ ...base, chatAccountId: 'acc-2' })).toEqual({ ...snapshot, replyToEmailId: null });
  });

  it('sends nothing when the composer has no text of the user', () => {
    expect(draftForTurn({ ...base, snapshot: null })).toBeNull();
  });
});

describe('draftChipScope', () => {
  it('says the thread travels with a reply', () => {
    expect(draftChipScope('reply', true)).toBe('threadAndReply');
    expect(draftChipScope('replyAll', true)).toBe('threadAndReply');
  });

  it('says only the reply travels when its thread does not', () => {
    expect(draftChipScope('reply', false)).toBe('reply');
  });

  it('says a new email is a new email', () => {
    expect(draftChipScope('new', false)).toBe('newEmail');
  });
});

describe('planDraftOffer', () => {
  const base = { kind: 'reply' as const, hasUserText: true, disabled: false, sameAccount: true };

  it('offers nothing without a composer on screen', () => {
    expect(planDraftOffer({ ...base, kind: null }).shown).toBe(false);
  });

  it('offers nothing while the composer holds only the signature', () => {
    expect(planDraftOffer({ ...base, hasUserText: false }).shown).toBe(false);
  });

  it('offers an active reply that carries its thread in place of the open one', () => {
    expect(planDraftOffer(base)).toEqual({ shown: true, active: true, scope: 'threadAndReply', carriesThread: true });
  });

  it('keeps a switched-off draft on screen and gives the open thread back', () => {
    expect(planDraftOffer({ ...base, disabled: true })).toEqual({
      shown: true,
      active: false,
      scope: 'threadAndReply',
      carriesThread: false,
    });
  });

  it('says only the reply travels when it is from another account', () => {
    expect(planDraftOffer({ ...base, sameAccount: false }).scope).toBe('reply');
  });

  it('offers a new email without any thread', () => {
    expect(planDraftOffer({ ...base, kind: 'new' })).toEqual({
      shown: true,
      active: true,
      scope: 'newEmail',
      carriesThread: false,
    });
  });
});
