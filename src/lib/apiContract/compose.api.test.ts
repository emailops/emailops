// Contract: the draft and translation commands are called from api.ts with the argument
// names their Rust signatures declare (see ./contract.ts).
import { describe, expect, it } from 'vitest';
import { itMatchesRustArguments, rustStructFields, tsInterfaceFields } from './contract';

describe('api.ts ↔ Rust: draft, outbox and translation commands', () => {
  itMatchesRustArguments(['drafts.rs', 'outbox.rs', 'translation.rs']);
});

describe('draft payloads have the shape the frontend types declare', () => {
  it('a stored draft is a Draft', () => {
    expect(rustStructFields('models/mod.rs', 'Draft')).toEqual(tsInterfaceFields('Draft'));
    expect(rustStructFields('models/mod.rs', 'DraftAttachment')).toEqual(tsInterfaceFields('DraftAttachment'));
  });

  it('save_draft reads a SaveDraftRequest', () => {
    expect(rustStructFields('models/mod.rs', 'SaveDraftRequest')).toEqual(
      tsInterfaceFields('SaveDraftRequest', 'src/lib/api.ts'),
    );
  });
});

describe('outbox payloads have the shape the frontend types declare', () => {
  it('a queued message is an OutgoingMessage', () => {
    expect(rustStructFields('models/outbox.rs', 'OutgoingMessage')).toEqual(
      tsInterfaceFields('OutgoingMessage', 'src/lib/api.ts'),
    );
  });

  it('a listed outbox row is an OutboxEntry', () => {
    expect(rustStructFields('models/outbox.rs', 'OutboxEntry')).toEqual(
      tsInterfaceFields('OutboxEntry', 'src/lib/api.ts'),
    );
  });
});
