// The "what's new" dialog shown once on the first launch after an update,
// in the UI language. Release notes are the real bundled files.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getBuildInfo: vi.fn(),
  getPref: vi.fn(),
  setPref: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn() }));

import { i18n, initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { PREF_RELEASE_NOTES_SEEN_VERSION, RELEASE_NOTES } from '@/lib/releaseNotes';
import { ReleaseNotesDialog } from './ReleaseNotesDialog';

// Any version that ships notes; the dialog is exercised against real files.
const VERSION = Object.keys(RELEASE_NOTES)[0] as string;

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getBuildInfo).mockResolvedValue({ version: VERSION, commit: null, isRelease: true });
  vi.mocked(api.setPref).mockResolvedValue(undefined);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  act(() => root.unmount());
  container.remove();
  await i18n.changeLanguage('en');
});

async function render(onboardingCompleted: boolean | null) {
  await act(async () => {
    root.render(<ReleaseNotesDialog onboardingCompleted={onboardingCompleted} />);
  });
}

function dialog() {
  return document.querySelector('[data-testid="release-notes"]');
}

describe('ReleaseNotesDialog', () => {
  it('shows the notes of the new version after an upgrade', async () => {
    vi.mocked(api.getPref).mockResolvedValue('0.0.1');
    await render(true);
    expect(dialog()?.textContent).toContain(firstHeading(RELEASE_NOTES[VERSION]?.en));
  });

  it('shows the notes in the UI language', async () => {
    await i18n.changeLanguage('es');
    vi.mocked(api.getPref).mockResolvedValue('0.0.1');
    await render(true);
    expect(dialog()?.textContent).toContain(firstHeading(RELEASE_NOTES[VERSION]?.es));
  });

  it('records the version as seen when closed, and closes', async () => {
    vi.mocked(api.getPref).mockResolvedValue('0.0.1');
    await render(true);
    expect(api.setPref).not.toHaveBeenCalled();

    const button = document.querySelector<HTMLButtonElement>('[data-testid="release-notes-close"]');
    await act(async () => {
      button?.click();
    });

    expect(api.setPref).toHaveBeenCalledWith(PREF_RELEASE_NOTES_SEEN_VERSION, VERSION);
    expect(dialog()).toBeNull();
  });

  it('stays closed when this version was already seen', async () => {
    vi.mocked(api.getPref).mockResolvedValue(VERSION);
    await render(true);
    expect(dialog()).toBeNull();
  });

  it('stays closed on a fresh install and records the version silently', async () => {
    vi.mocked(api.getPref).mockResolvedValue(null);
    await render(false);
    expect(dialog()).toBeNull();
    expect(api.setPref).toHaveBeenCalledWith(PREF_RELEASE_NOTES_SEEN_VERSION, VERSION);
  });

  it('waits until the onboarding check has resolved', async () => {
    vi.mocked(api.getPref).mockResolvedValue('0.0.1');
    await render(null);
    expect(api.getPref).not.toHaveBeenCalled();
    expect(dialog()).toBeNull();
  });
});

/** First `### heading` of a notes file — a stable, language-specific probe. */
function firstHeading(markdown: string | undefined): string {
  return /^###\s+(.+)$/m.exec(markdown ?? '')?.[1] ?? '<<missing heading>>';
}
