// Contract: the attachment, Lens and task commands are called from api.ts with the argument
// names their Rust signatures declare (see ./contract.ts).
import { describe } from 'vitest';
import { itMatchesRustArguments } from './contract';

describe('api.ts ↔ Rust: attachment, Lens and task commands', () => {
  itMatchesRustArguments(['attachments.rs', 'lenses.rs']);
});
