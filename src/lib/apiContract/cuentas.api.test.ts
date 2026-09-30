// Contract: the account and sync commands are called from api.ts with the argument
// names their Rust signatures declare (see ./contract.ts).
import { describe } from 'vitest';
import { itMatchesRustArguments } from './contract';

describe('api.ts ↔ Rust: account and sync commands', () => {
  itMatchesRustArguments(['accounts.rs', 'system.rs', 'connectivity.rs']);
});
