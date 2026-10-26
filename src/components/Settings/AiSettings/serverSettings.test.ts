import { describe, expect, it } from 'vitest';
import { aiRunsRemotely, contextReportLines, disclosureHost, sendsIndexToOpenRouter } from './serverSettings';

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

describe('aiRunsRemotely', () => {
  it('is false for the local backends and for a server on this machine', () => {
    expect(aiRunsRemotely(null, null)).toBe(false);
    expect(aiRunsRemotely('llamacpp', null)).toBe(false);
    expect(aiRunsRemotely('ollama', null)).toBe(false);
    expect(aiRunsRemotely('openai_compatible', 'http://localhost:1234/v1')).toBe(false);
  });

  it('is true for OpenRouter and for a server on another machine', () => {
    expect(aiRunsRemotely('openrouter', null)).toBe(true);
    expect(aiRunsRemotely('openai_compatible', 'https://llm.example.com/v1')).toBe(true);
  });
});

describe('contextReportLines', () => {
  const report = (over: Partial<Parameters<typeof contextReportLines>[0]>) => ({
    window: 32768,
    source: 'server' as const,
    served: 32768,
    configured: 0,
    recommended: 16384,
    belowRecommended: false,
    ...over,
  });
  const keys = (r: Parameters<typeof contextReportLines>[0]) => contextReportLines(r).map((l) => l.key);

  it('names the window the server loaded, and says when it caps the one set here', () => {
    expect(keys(report({}))).toEqual(['settings:server.contextLoaded']);
    expect(keys(report({ window: 8192, served: 8192, configured: 32768 }))).toEqual(['settings:server.contextCapped']);
  });

  it('says when the window set here is used, and whether the server confirmed it', () => {
    expect(keys(report({ source: 'configured', configured: 32768, served: 65536 }))).toEqual([
      'settings:server.contextConfigured',
    ]);
    expect(keys(report({ source: 'configured', configured: 32768, served: null }))).toEqual([
      'settings:server.contextNotReported',
    ]);
  });

  it('says when nothing is known and the default is assumed', () => {
    expect(keys(report({ source: 'assumed', window: 8192, served: null, belowRecommended: true }))).toEqual([
      'settings:server.contextAssumed',
      'settings:server.contextBelowRecommended',
    ]);
  });

  it('marks only the recommendation as a warning', () => {
    const lines = contextReportLines(report({ window: 4096, served: 4096, belowRecommended: true }));
    expect(lines.map((l) => l.warning)).toEqual([false, true]);
    expect(lines[1].params).toEqual({ recommended: 16384 });
  });
});
