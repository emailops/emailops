// Contract: the classification and filter commands are called from api.ts with the argument
// names their Rust signatures declare (see ./contract.ts).
import { describe } from 'vitest';
import { itMatchesRustArguments } from './contract';

describe('api.ts ↔ Rust: classification and filter commands', () => {
  itMatchesRustArguments(['classification.rs', 'filters.rs']);
});
