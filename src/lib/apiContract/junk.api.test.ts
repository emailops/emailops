// Contract: the junk commands are called from api.ts with the argument
// names their Rust signatures declare (see ./contract.ts).
import { describe } from 'vitest';
import { itMatchesRustArguments } from './contract';

describe('api.ts ↔ Rust: junk commands', () => {
  itMatchesRustArguments(['junk.rs']);
});
