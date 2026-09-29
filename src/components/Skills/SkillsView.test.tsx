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
const saveSkill = vi.fn<(name: string, content: string, base: string) => Promise<string>>();
const createSkill = vi.fn<(name: string) => Promise<void>>();
const setSkillEnabled = vi.fn<(name: string, enabled: boolean) => Promise<void>>();
const deleteSkill = vi.fn<(name: string) => Promise<void>>();
vi.mock('@/lib/api', () => ({
  listSkills: () => listSkills(),
  readSkill: (n: string) => readSkill(n),
  saveSkill: (n: string, c: string, b: string) => saveSkill(n, c, b),
  createSkill: (n: string) => createSkill(n),
  setSkillEnabled: (n: string, e: boolean) => setSkillEnabled(n, e),
  deleteSkill: (n: string) => deleteSkill(n),
}));

const errorText = vi.fn((e: unknown) =>
  typeof e === 'object' && e !== null && 'code' in e ? `translated:${(e as { code: string }).code}` : String(e),
);
vi.mock('@/lib/errors', () => ({ errorText: (e: unknown) => errorText(e) }));

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
  saveSkill.mockImplementation((n) => Promise.resolve(n));
  createSkill.mockResolvedValue();
  setSkillEnabled.mockResolvedValue();
  deleteSkill.mockResolvedValue();
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
    expect(saveSkill).toHaveBeenCalledWith('beta', 'edited text', '---\nname: beta\n---\nbody of beta');
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

  it('keeps what was typed while a save was in flight', async () => {
    listSkills.mockResolvedValue(overview([{ name: 'alpha' }]));
    let finish: (name: string) => void = () => {};
    saveSkill.mockImplementation(() => new Promise<string>((resolve) => (finish = resolve)));
    await render();
    await type(q('skill-editor'), 'first');
    await click(q('skill-save'));
    await type(q('skill-editor'), 'first and more');
    await act(async () => finish('alpha'));
    expect((q('skill-editor') as HTMLTextAreaElement).value).toBe('first and more');
    expect(container.textContent).toContain('settings:skills.view.unsaved');
  });

  it('follows the skill when the saved name renames it', async () => {
    listSkills.mockResolvedValue(overview([{ name: 'weekly-report' }]));
    await render();
    await type(q('skill-editor'), '---\nname: weekly-email-summary\n---\nsteps');
    saveSkill.mockResolvedValue('weekly-email-summary');
    listSkills.mockResolvedValue(overview([{ name: 'weekly-email-summary' }]));
    await click(q('skill-save'));
    expect(q('skill-row-weekly-email-summary')).not.toBeNull();
    expect(container.textContent).toContain('weekly-email-summary/SKILL.md');
    expect((q('skill-editor') as HTMLTextAreaElement).value).toContain('name: weekly-email-summary');
    expect(container.textContent).not.toContain('settings:skills.view.unsaved');
  });

  it('offers to reload a skill that changed on disk', async () => {
    listSkills.mockResolvedValue(overview([{ name: 'alpha' }]));
    saveSkill.mockRejectedValue(new Error('alpha/SKILL.md changed on disk since you opened it'));
    await render();
    await type(q('skill-editor'), 'mine');
    await click(q('skill-save'));
    readSkill.mockResolvedValue('theirs');
    await click(q('skill-reload'));
    expect((q('skill-editor') as HTMLTextAreaElement).value).toBe('theirs');
    expect(q('skills-error')).toBeNull();
  });

  it('deletes a skill only after the user confirms', async () => {
    listSkills.mockResolvedValue(overview([{ name: 'alpha' }, { name: 'beta' }]));
    await render();
    await click(q('skill-delete'));
    expect(deleteSkill).not.toHaveBeenCalled();
    await click(q('skill-delete-cancel'));
    expect(q('skill-delete-confirm')).toBeNull();

    await click(q('skill-delete'));
    listSkills.mockResolvedValue(overview([{ name: 'beta' }]));
    await click(q('skill-delete-confirm'));
    expect(deleteSkill).toHaveBeenCalledWith('alpha');
    expect(q('skill-row-alpha')).toBeNull();
    // The next skill opens instead of an editor pointing at a deleted file.
    expect(readSkill).toHaveBeenLastCalledWith('beta');
  });

  it('shows why a skill in the folder failed to load, translated', async () => {
    listSkills.mockResolvedValue({
      ...overview([]),
      errors: [
        {
          path: '/data/skills/broken/SKILL.md',
          message: 'SKILL.md must start with a `---` frontmatter block',
          code: 'skill_no_frontmatter',
          params: {},
        },
      ],
    });
    await render();
    expect(errorText).toHaveBeenCalledWith(expect.objectContaining({ code: 'skill_no_frontmatter' }));
    expect(container.textContent).toContain('translated:skill_no_frontmatter');
  });
});
