// translationStore actions and event wiring (translationStore.test.ts covers
// the pure event reducers).
//
// `detect` and `translate` reserve their slot before awaiting so a re-render
// cannot fire twice, release it when the command fails, and `translate` reuses
// a cached translation instead of calling the model again.

import { beforeEach, describe, expect, it, vi } from 'vitest';

type Handler = (event: { payload: unknown }) => void;
const listeners = vi.hoisted(() => ({ byName: {} as Record<string, Handler> }));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((name: string, handler: Handler) => {
    listeners.byName[name] = handler;
    return Promise.resolve(() => {});
  }),
}));

const api = vi.hoisted(() => ({
  detectEmailLanguage: vi.fn(),
  translateEmail: vi.fn(),
}));

vi.mock('@/lib/api', () => api);

const addLog = vi.hoisted(() => vi.fn());
vi.mock('@/stores/logStore', () => ({ useLogStore: { getState: () => ({ addLog }) } }));

import { listen } from '@tauri-apps/api/event';
import { initTranslationListeners, useTranslationStore } from './translationStore';

const EMPTY = {
  detectedByEmail: {},
  translations: {},
  showTranslated: {},
  pendingDetect: {},
  pendingTranslate: {},
  errorByEmail: {},
};

beforeEach(() => {
  api.detectEmailLanguage.mockReset();
  api.translateEmail.mockReset();
  addLog.mockReset();
  useTranslationStore.setState({ ...EMPTY });
});

describe('translationStore.detect', () => {
  it('records the request id of a started detection', async () => {
    api.detectEmailLanguage.mockResolvedValue('req-1');

    await useTranslationStore.getState().detect('e1');

    expect(useTranslationStore.getState().pendingDetect).toEqual({ e1: 'req-1' });
  });

  it('does not ask twice for an email already detected or in flight', async () => {
    useTranslationStore.setState({
      detectedByEmail: { e1: { language: 'es', needsTranslation: true } },
      pendingDetect: { e2: '' },
    });

    await useTranslationStore.getState().detect('e1');
    await useTranslationStore.getState().detect('e2');

    expect(api.detectEmailLanguage).not.toHaveBeenCalled();
  });

  it('releases the slot and logs at debug level when detection is unavailable', async () => {
    api.detectEmailLanguage.mockRejectedValue(new Error('AI disabled'));

    await useTranslationStore.getState().detect('e1');

    expect(useTranslationStore.getState().pendingDetect).toEqual({});
    expect(addLog).toHaveBeenCalledWith('debug', 'ai', expect.stringContaining('AI disabled'));
  });
});

describe('translationStore.translate', () => {
  it('starts a translation and clears an earlier error', async () => {
    useTranslationStore.setState({ errorByEmail: { e1: 'old failure' } });
    api.translateEmail.mockResolvedValue('req-7');

    await useTranslationStore.getState().translate('e1');

    const s = useTranslationStore.getState();
    expect(s.pendingTranslate).toEqual({ e1: 'req-7' });
    expect(s.errorByEmail.e1).toBeNull();
  });

  it('shows a cached translation without calling the model again', async () => {
    useTranslationStore.setState({
      translations: { e1: { text: 'Hola', targetLanguage: 'Spanish', truncated: false } },
    });

    await useTranslationStore.getState().translate('e1');

    expect(api.translateEmail).not.toHaveBeenCalled();
    expect(useTranslationStore.getState().showTranslated.e1).toBe(true);
  });

  it('ignores a second request while one is in flight', async () => {
    useTranslationStore.setState({ pendingTranslate: { e1: '' } });

    await useTranslationStore.getState().translate('e1');

    expect(api.translateEmail).not.toHaveBeenCalled();
  });

  it('keeps the failure on the email and logs it when the translation cannot start', async () => {
    api.translateEmail.mockRejectedValue(new Error('model not loaded'));

    await useTranslationStore.getState().translate('e1');

    const s = useTranslationStore.getState();
    expect(s.pendingTranslate).toEqual({});
    expect(s.errorByEmail.e1).toContain('model not loaded');
    expect(addLog).toHaveBeenCalledWith('error', 'ai', expect.stringContaining('model not loaded'));
  });
});

describe('translationStore.toggle', () => {
  it('flips between the original and the translation', () => {
    useTranslationStore.getState().toggle('e1');
    expect(useTranslationStore.getState().showTranslated.e1).toBe(true);
    useTranslationStore.getState().toggle('e1');
    expect(useTranslationStore.getState().showTranslated.e1).toBe(false);
  });
});

describe('initTranslationListeners', () => {
  it('subscribes once and feeds the three backend events into the store', () => {
    initTranslationListeners();
    initTranslationListeners();
    expect(listen).toHaveBeenCalledTimes(3);
    expect(Object.keys(listeners.byName).sort()).toEqual([
      'email-translated',
      'language-detected',
      'translation-failed',
    ]);

    useTranslationStore.setState({ pendingDetect: { e1: 'd1' }, pendingTranslate: { e1: 't1', e2: 't2' } });
    listeners.byName['language-detected']({
      payload: { requestId: 'd1', emailId: 'e1', language: 'fr', preferredLanguage: 'en', needsTranslation: true },
    });
    listeners.byName['email-translated']({
      payload: { requestId: 't1', emailId: 'e1', text: 'Hello', targetLanguage: 'English', truncated: false },
    });
    listeners.byName['translation-failed']({ payload: { requestId: 't2', emailId: 'e2', error: 'too long' } });

    const s = useTranslationStore.getState();
    expect(s.detectedByEmail.e1).toEqual({ language: 'fr', needsTranslation: true });
    expect(s.translations.e1?.text).toBe('Hello');
    expect(s.errorByEmail.e2).toBe('too long');
    expect(s.pendingTranslate).toEqual({});
  });
});
