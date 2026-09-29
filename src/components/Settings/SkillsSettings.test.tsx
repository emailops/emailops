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
  setEnabled: () => Promise.resolve(),
}));
vi.mock('@/stores/featureToggleStore', () => ({
  useSkillsEnabledStore: () => skillsStore,
}));

const listSkills = vi.fn<() => Promise<SkillsOverview>>();
const openSkillsFolder = vi.fn(() => Promise.resolve());
vi.mock('@/lib/api', () => ({
  listSkills: () => listSkills(),
  openSkillsFolder: () => openSkillsFolder(),
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
    listSkills.mockResolvedValue({ enabled: true, dir: '/data/skills', skills: [], errors: [] });
    await render();
    expect(container.textContent).toContain('settings:skills.empty');
  });

  it('opens the skills folder and re-reads it on demand', async () => {
    listSkills.mockResolvedValue({ enabled: true, dir: '/data/skills', skills: [], errors: [] });
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
