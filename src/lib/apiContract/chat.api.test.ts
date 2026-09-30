// Contract: the chat, memory and prompt commands are called from api.ts with the argument
// names their Rust signatures declare (see ./contract.ts).
import { describe } from 'vitest';
import { itMatchesRustArguments } from './contract';

describe('api.ts ↔ Rust: chat, memory and prompt commands', () => {
  itMatchesRustArguments(['chat.rs', 'memory.rs', 'prompts.rs']);
});
