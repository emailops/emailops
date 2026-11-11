import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SkillsOverview } from '@/lib/api';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const addLog = vi.fn();
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: typeof addLog }) => unknown) => selector({ addLog }),
}));

// One stable object, like the real zustand store: a fresh `refresh` per render
// would re-run the panel's load effect on every render.
const skillsStore = vi.hoisted(() => ({
  enabled: true,
  isLoading: false,
  refresh: () => Promise.resolve(),
  setEnabled: (_enabled: boolean) => Promise.resolve(),
}));
vi.mock('@/stores/featureToggleStore', () => ({
  useSkillsEnabledStore: () => skillsStore,
}));

const listSkills = vi.fn<() => Promise<SkillsOverview>>();
const openSkillsFolder = vi.fn(() => Promise.resolve());
const setBundledSkillEnabled = vi.fn<(name: string, enabled: boolean) => Promise<void>>();
vi.mock('@/lib/api', () => ({
  listSkills: () => listSkills(),
  openSkillsFolder: () => openSkillsFolder(),
  setBundledSkillEnabled: (n: string, e: boolean) => setBundledSkillEnabled(n, e),
}));

const errorText = vi.fn((e: unknown) =>
  typeof e === 'object' && e !== null && 'code' in e ? `translated:${(e as { code: string }).code}` : String(e),
);
vi.mock('@/lib/errors', () => ({ errorText: (e: unknown) => errorText(e) }));

import { SkillsSettings } from './SkillsSettings';

let container: HTMLDivElement;
let root: Root;

async function render() {
  await act(async () => {
    root.render(<SkillsSettings />);
  });
}

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  skillsStore.enabled = true;
});

const q = (id: string) => container.querySelector<HTMLElement>(`[data-testid="${id}"]`);

async function click(el: HTMLElement | null) {
  await act(async () => {
    el?.click();
  });
}

function withBundled(enabled: boolean, bundledOn = false): SkillsOverview {
  return {
    enabled,
    dir: '/data/skills',
    skills: [],
    bundled: [{ name: 'inbox-triage', description: 'Triage the inbox.', enabled: bundledOn, shadowed: false }],
    errors: [],
  };
}

describe('SkillsSettings: bundled skills', () => {
  beforeEach(() => {
    setBundledSkillEnabled.mockResolvedValue();
  });

  it('shows them with the feature off', async () => {
    skillsStore.enabled = false;
    listSkills.mockResolvedValue(withBundled(false));
    await render();
    expect(container.textContent).toContain('/inbox-triage');
    expect(container.textContent).toContain('Triage the inbox.');
    expect(q('bundled-toggle-inbox-triage')?.getAttribute('aria-checked')).toBe('false');
  });

  it('switches one straight on while the feature is on', async () => {
    listSkills.mockResolvedValue(withBundled(true));
    await render();
    await click(q('bundled-toggle-inbox-triage'));
    expect(setBundledSkillEnabled).toHaveBeenCalledWith('inbox-triage', true);
    expect(q('skills-offer')).toBeNull();
  });

  it('with the feature off, offers to turn it on before switching anything', async () => {
    skillsStore.enabled = false;
    listSkills.mockResolvedValue(withBundled(false));
    await render();
    await click(q('bundled-toggle-inbox-triage'));
    expect(q('skills-offer')).not.toBeNull();
    expect(setBundledSkillEnabled).not.toHaveBeenCalled();
  });

  it('accepting the offer turns the feature and the skill on', async () => {
    const setEnabled = vi.spyOn(skillsStore, 'setEnabled');
    skillsStore.enabled = false;
    listSkills.mockResolvedValue(withBundled(false));
    await render();
    await click(q('bundled-toggle-inbox-triage'));
    await click(q('skills-offer-yes'));
    expect(setEnabled).toHaveBeenCalledWith(true);
    expect(setBundledSkillEnabled).toHaveBeenCalledWith('inbox-triage', true);
    expect(q('skills-offer')).toBeNull();
    setEnabled.mockRestore();
  });

  it('declining the offer changes nothing', async () => {
    const setEnabled = vi.spyOn(skillsStore, 'setEnabled');
    skillsStore.enabled = false;
    listSkills.mockResolvedValue(withBundled(false));
    await render();
    await click(q('bundled-toggle-inbox-triage'));
    await click(q('skills-offer-no'));
    expect(setEnabled).not.toHaveBeenCalled();
    expect(setBundledSkillEnabled).not.toHaveBeenCalled();
    expect(q('skills-offer')).toBeNull();
    setEnabled.mockRestore();
  });

  it('switching one off never asks', async () => {
    skillsStore.enabled = false;
    listSkills.mockResolvedValue(withBundled(false, true));
    await render();
    await click(q('bundled-toggle-inbox-triage'));
    expect(q('skills-offer')).toBeNull();
    expect(setBundledSkillEnabled).toHaveBeenCalledWith('inbox-triage', false);
  });
});

describe('SkillsSettings', () => {
  it('lists each skill as its slash command with its description', async () => {
    listSkills.mockResolvedValue({
      enabled: true,
      dir: '/data/skills',
      skills: [
        {
          name: 'weekly-summary',
          description: 'Weekly recap by client.',
          path: '/data/skills/weekly-summary/SKILL.md',
          enabled: true,
        },
      ],
      bundled: [],
      errors: [],
    });
    await render();
    expect(container.textContent).toContain('/weekly-summary');
    expect(container.textContent).toContain('Weekly recap by client.');
    expect(container.textContent).toContain('/data/skills');
  });

  it('shows why a skill failed to load', async () => {
    listSkills.mockResolvedValue({
      enabled: true,
      dir: '/data/skills',
      skills: [],
      bundled: [],
      errors: [
        {
          path: '/data/skills/Bad/SKILL.md',
          message: 'the frontmatter has no `name`',
          code: 'skill_no_name',
          params: {},
        },
      ],
    });
    await render();
    expect(container.textContent).toContain('/data/skills/Bad/SKILL.md');
    // Rendered through errorText, which translates `errors:codes.<code>`.
    expect(errorText).toHaveBeenCalledWith(expect.objectContaining({ code: 'skill_no_name' }));
    expect(container.textContent).toContain('translated:skill_no_name');
  });

  it('says how to add one when the folder is empty', async () => {
    listSkills.mockResolvedValue({ enabled: true, dir: '/data/skills', skills: [], bundled: [], errors: [] });
    await render();
    expect(container.textContent).toContain('settings:skills.empty');
  });

  it('opens the skills folder and re-reads it on demand', async () => {
    listSkills.mockResolvedValue({ enabled: true, dir: '/data/skills', skills: [], bundled: [], errors: [] });
    await render();
    const open = container.querySelector('[data-testid="skills-open-folder"]') as HTMLButtonElement;
    await act(async () => open.click());
    expect(openSkillsFolder).toHaveBeenCalledTimes(1);
    // Opening creates the folder when missing, so the listing is re-read.
    expect(listSkills).toHaveBeenCalledTimes(2);
    const reload = container.querySelector('[data-testid="skills-reload"]') as HTMLButtonElement;
    await act(async () => reload.click());
    expect(listSkills).toHaveBeenCalledTimes(3);
  });

  it('logs and shows a listing failure instead of an empty list', async () => {
    listSkills.mockRejectedValue(new Error('disk on fire'));
    await render();
    expect(container.textContent).toContain('disk on fire');
    expect(addLog).toHaveBeenCalledWith('error', 'ai', expect.stringContaining('disk on fire'));
  });
});
