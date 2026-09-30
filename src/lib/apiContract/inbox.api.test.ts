// Contract: the email commands are called from api.ts with the argument
// names their Rust signatures declare (see ./contract.ts).
import { describe } from 'vitest';
import { itMatchesRustArguments } from './contract';

describe('api.ts ↔ Rust: email commands', () => {
  itMatchesRustArguments(['emails.rs', 'trusted_senders.rs']);
});
