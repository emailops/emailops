// Step order and KV-cache facts moved to the backend with their tests
// (`src-tauri/src/services/chat/trace_steps.rs`); only formatting is left here.
import { describe, expect, it } from 'vitest';
import { formatLatency, tokensPerSecond } from './reasoningTrace';

describe('formatLatency', () => {
  it('renders sub-second durations in ms', () => {
    expect(formatLatency(500)).toBe('500ms');
  });

  it('renders durations >= 1s with one decimal', () => {
    expect(formatLatency(30000)).toBe('30.0s');
  });
});

describe('tokensPerSecond', () => {
  it('divides tokens by seconds', () => {
    expect(tokensPerSecond(300, 30000)).toBe(10);
  });

  it('returns 0 when the duration is zero to avoid Infinity', () => {
    expect(tokensPerSecond(50, 0)).toBe(0);
  });

  it('returns 0 when token count is missing', () => {
    expect(tokensPerSecond(null, 1000)).toBe(0);
  });
});
