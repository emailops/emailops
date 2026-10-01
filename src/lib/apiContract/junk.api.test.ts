// Contract: the junk and sender-control (block sender, unsubscribe) commands
// are called from api.ts with the argument names their Rust signatures declare
// (see ./contract.ts), and their payloads have the shape the frontend declares.
import { describe, expect, it } from 'vitest';
import { itMatchesRustArguments, rustStructFields, tsInterfaceFields } from './contract';

describe('api.ts ↔ Rust: junk and sender-control commands', () => {
  itMatchesRustArguments(['junk.rs', 'sender_controls.rs']);
});

describe('sender-control payloads have the shape the frontend types declare', () => {
  it.each([
    ['services/sender_controls.rs', 'SenderStatus'],
    ['services/sender_controls.rs', 'SenderMoveReport'],
    ['services/unsubscribe.rs', 'UnsubscribeOption'],
    ['models/mod.rs', 'BlockedSender'],
  ])('%s %s', (file, name) => {
    expect(rustStructFields(file, name)).toEqual(tsInterfaceFields(name));
  });
});
