// Contract: the contacts and dashboard commands are called from api.ts with the argument
// names their Rust signatures declare (see ./contract.ts).
import { describe } from 'vitest';
import { itMatchesRustArguments } from './contract';

describe('api.ts ↔ Rust: contacts and dashboard commands', () => {
  itMatchesRustArguments(['contacts.rs', 'dashboard.rs']);
});
