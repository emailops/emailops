// Contract: the preference and security commands are called from api.ts with the argument
// names their Rust signatures declare (see ./contract.ts).
import { describe } from 'vitest';
import { itMatchesRustArguments } from './contract';

describe('api.ts ↔ Rust: preference and security commands', () => {
  itMatchesRustArguments(['preferences.rs', 'security.rs']);
});
