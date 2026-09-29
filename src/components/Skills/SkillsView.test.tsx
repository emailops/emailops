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

const listSkills = vi.fn<() => Promise<SkillsOverview>>();
const readSkill = vi.fn<(name: string) => Promise<string>>();
const saveSkill = vi.fn<(name: string, content: string) => Promise<void>>();
const createSkill = vi.fn<(name: string) => Promise<void>>();
const setSkillEnabled = vi.fn<(name: string, enabled: boolean) => Promise<void>>();
vi.mock('@/lib/api', () => ({
  listSkills: () => listSkills(),
  readSkill: (n: string) => readSkill(n),
  saveSkill: (n: string, c: string) => saveSkill(n, c),
  createSkill: (n: string) => createSkill(n),
  setSkillEnabled: (n: string, e: boolean) => setSkillEnabled(n, e),
}));

import { SkillsView } from './SkillsView';

let container: HTMLDivElement;
let root: Root;

function overview(skills: { name: string; enabled?: boolean }[]): SkillsOverview {
  return {
    enabled: true,
    dir: '/data/skills',
    skills: skills.map((s) => ({
      name: s.name,
      description: `${s.name} description`,
      path: `/data/skills/${s.name}/SKILL.md`,
      enabled: s.enabled ?? true,
    })),
    errors: [],
  };
}

const q = (id: string) => container.querySelector<HTMLElement>(`[data-testid="${id}"]`);

async function render() {
  await act(async () => {
    root.render(<SkillsView />);
  });
}

async function click(el: HTMLElement | null) {
  await act(async () => {
    el?.click();
  });
}

async function type(el: HTMLElement | null, value: string) {
  const input = el as HTMLInputElement | HTMLTextAreaElement;
  const proto = input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  await act(async () => {
    Object.getOwnPropertyDescriptor(proto, 'value')?.set?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
  readSkill.mockImplementation((n) => Promise.resolve(`---\nname: ${n}\n---\nbody of ${n}`));
  saveSkill.mockResolvedValue();
  createSkill.mockResolvedValue();
  setSkillEnabled.mockResolvedValue();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('SkillsView', () => {
  it('lists every skill and opens the first one in the editor', async () => {
    listSkills.mockResolvedValue(overview([{ name: 'alpha' }, { name: 'beta', enabled: false }]));
    await render();
    expect(q('skill-row-alpha')?.textContent).toContain('alpha description');
    expect(q('skill-row-beta')).not.toBeNull();
    expect((q('skill-editor') as HTMLTextAreaElement).value).toContain('body of alpha');
  });

  it('switches a skill off from its toggle', async () => {
    listSkills.mockResolvedValue(overview([{ name: 'alpha' }]));
    await render();
    expect(q('skill-toggle-alpha')?.getAttribute('aria-checked')).toBe('true');
    listSkills.mockResolvedValue(overview([{ name: 'alpha', enabled: false }]));
    await click(q('skill-toggle-alpha'));
    expect(setSkillEnabled).toHaveBeenCalledWith('alpha', false);
    expect(q('skill-toggle-alpha')?.getAttribute('aria-checked')).toBe('false');
  });

  it('loads the selected skill into the editor and saves the edit', async () => {
    listSkills.mockResolvedValue(overview([{ name: 'alpha' }, { name: 'beta' }]));
    await render();
    await click(q('skill-row-beta'));
    expect(readSkill).toHaveBeenLastCalledWith('beta');
    await type(q('skill-editor'), 'edited text');
    await click(q('skill-save'));
    expect(saveSkill).toHaveBeenCalledWith('beta', 'edited text');
  });

  it('keeps the edit and shows why a save was rejected', async () => {
    listSkills.mockResolvedValue(overview([{ name: 'alpha' }]));
    saveSkill.mockRejectedValue(new Error('the frontmatter has no `description`'));
    await render();
    await type(q('skill-editor'), 'broken');
    await click(q('skill-save'));
    expect(q('skills-error')?.textContent).toContain('description');
    expect((q('skill-editor') as HTMLTextAreaElement).value).toBe('broken');
  });

  it('creates a new skill and opens it', async () => {
    listSkills.mockResolvedValue(overview([]));
    await render();
    await click(q('skill-new'));
    await type(q('skill-new-name'), 'weekly-digest');
    listSkills.mockResolvedValue(overview([{ name: 'weekly-digest' }]));
    await click(q('skill-create'));
    expect(createSkill).toHaveBeenCalledWith('weekly-digest');
    expect(readSkill).toHaveBeenLastCalledWith('weekly-digest');
    expect((q('skill-editor') as HTMLTextAreaElement).value).toContain('body of weekly-digest');
  });
});
