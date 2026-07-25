import { describe, expect, it } from 'vitest';
import {
  reduceEmailTranslated,
  reduceLanguageDetected,
  reduceTranslationFailed,
  type TranslationStoreState,
} from './translationStore';

function baseState(overrides: Partial<TranslationStoreState> = {}): TranslationStoreState {
  return {
    detectedByEmail: {},
    translations: {},
    showTranslated: {},
    pendingDetect: {},
    pendingTranslate: {},
    errorByEmail: {},
    ...overrides,
  };
}

describe('reduceLanguageDetected', () => {
  it('stores the detection and clears the pending slot', () => {
    const state = baseState({ pendingDetect: { 'eml-1': 'req-1' } });
    const next = reduceLanguageDetected(state, {
      requestId: 'req-1',
      emailId: 'eml-1',
      language: 'es',
      preferredLanguage: 'en',
      needsTranslation: true,
    });
    expect(next.detectedByEmail?.['eml-1']).toEqual({ language: 'es', needsTranslation: true });
    expect(next.pendingDetect).toEqual({});
  });

  it('keeps other pending detections intact', () => {
    const state = baseState({ pendingDetect: { 'eml-1': 'req-1', 'eml-2': 'req-2' } });
    const next = reduceLanguageDetected(state, {
      requestId: 'req-1',
      emailId: 'eml-1',
      language: 'und',
      preferredLanguage: 'en',
      needsTranslation: false,
    });
    expect(next.pendingDetect).toEqual({ 'eml-2': 'req-2' });
    expect(next.detectedByEmail?.['eml-1']).toEqual({ language: 'und', needsTranslation: false });
  });
});

describe('reduceEmailTranslated', () => {
  const event = {
    requestId: 'req-9',
    emailId: 'eml-1',
    targetLanguage: 'English',
    text: 'Hello there.',
    truncated: false,
  };

  it('stores the translation, reveals it, and clears pending + error', () => {
    const state = baseState({
      pendingTranslate: { 'eml-1': 'req-9' },
      errorByEmail: { 'eml-1': 'old error' },
    });
    const next = reduceEmailTranslated(state, event);
    expect(next).not.toBeNull();
    expect(next?.translations?.['eml-1']).toEqual({
      text: 'Hello there.',
      targetLanguage: 'English',
      truncated: false,
    });
    expect(next?.showTranslated?.['eml-1']).toBe(true);
    expect(next?.pendingTranslate).toEqual({});
    expect(next?.errorByEmail?.['eml-1']).toBeNull();
  });

  it('ignores a stale event whose requestId does not match', () => {
    const state = baseState({ pendingTranslate: { 'eml-1': 'req-NEWER' } });
    expect(reduceEmailTranslated(state, event)).toBeNull();
  });

  it('ignores an event for an email with nothing pending', () => {
    expect(reduceEmailTranslated(baseState(), event)).toBeNull();
  });
});

describe('reduceTranslationFailed', () => {
  it('records the error and clears the pending slot', () => {
    const state = baseState({ pendingTranslate: { 'eml-1': 'req-9' } });
    const next = reduceTranslationFailed(state, { requestId: 'req-9', emailId: 'eml-1', error: 'model exploded' });
    expect(next?.errorByEmail?.['eml-1']).toBe('model exploded');
    expect(next?.pendingTranslate).toEqual({});
  });

  it('ignores compose failures (empty emailId) — handled by compose components', () => {
    const state = baseState({ pendingTranslate: { 'eml-1': 'req-9' } });
    expect(reduceTranslationFailed(state, { requestId: 'req-9', emailId: '', error: 'x' })).toBeNull();
  });

  it('ignores stale failures whose requestId does not match', () => {
    const state = baseState({ pendingTranslate: { 'eml-1': 'req-NEWER' } });
    expect(reduceTranslationFailed(state, { requestId: 'req-9', emailId: 'eml-1', error: 'x' })).toBeNull();
  });
});
