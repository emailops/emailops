import { describe, expect, it } from 'vitest';
import { disclosureHost, sendsIndexToOpenRouter } from './serverSettings';

describe('disclosureHost', () => {
  it('asks nothing for a server on this machine', () => {
    for (const url of ['http://localhost:1234/v1', 'http://127.0.0.1:8080', 'http://127.4.5.6', 'http://[::1]:8000']) {
      expect(disclosureHost(url, null)).toBeNull();
    }
  });

  it('names a remote host the user has not confirmed', () => {
    expect(disclosureHost('https://llm.example.com/v1', null)).toBe('llm.example.com');
    expect(disclosureHost(' https://LLM.example.com/v1/ ', null)).toBe('llm.example.com');
  });

  it('asks again only when the remote host changes', () => {
    expect(disclosureHost('https://llm.example.com/v2', 'llm.example.com')).toBeNull();
    expect(disclosureHost('https://other.example.net/v1', 'llm.example.com')).toBe('other.example.net');
  });

  it('leaves a URL it cannot read to the backend check', () => {
    expect(disclosureHost('not a url', null)).toBeNull();
    expect(disclosureHost('', null)).toBeNull();
  });
});

describe('sendsIndexToOpenRouter', () => {
  it('is true for OpenRouter, and for a server whose index is computed there', () => {
    expect(sendsIndexToOpenRouter('openrouter', undefined)).toBe(true);
    expect(sendsIndexToOpenRouter('openai_compatible', 'openrouter')).toBe(true);
    expect(sendsIndexToOpenRouter('openai_compatible', 'server')).toBe(false);
    expect(sendsIndexToOpenRouter('openai_compatible', 'inapp')).toBe(false);
    expect(sendsIndexToOpenRouter('llamacpp', undefined)).toBe(false);
  });
});
