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
const setBundledSkillEnabled = vi.fn<(name: string, enabled: boolean) => Promise<void>>();
const readBundledSkill = vi.fn<(name: string) => Promise<string>>();
const copyBundledSkill = vi.fn<(name: string) => Promise<void>>();
vi.mock('@/lib/api', () => ({
  listSkills: () => listSkills(),
  readSkill: (n: string) => readSkill(n),
  saveSkill: (n: string, c: string, b: string) => saveSkill(n, c, b),
  createSkill: (n: string) => createSkill(n),
  setSkillEnabled: (n: string, e: boolean) => setSkillEnabled(n, e),
  deleteSkill: (n: string) => deleteSkill(n),
  setBundledSkillEnabled: (n: string, e: boolean) => setBundledSkillEnabled(n, e),
  readBundledSkill: (n: string) => readBundledSkill(n),
  copyBundledSkill: (n: string) => copyBundledSkill(n),
}));

const errorText = vi.fn((e: unknown) =>
  typeof e === 'object' && e !== null && 'code' in e ? `translated:${(e as { code: string }).code}` : String(e),
);
vi.mock('@/lib/errors', () => ({ errorText: (e: unknown) => errorText(e) }));

import { SkillsView } from './SkillsView';

let container: HTMLDivElement;
let root: Root;

type Bundled = { name: string; enabled?: boolean; shadowed?: boolean };

function overview(skills: { name: string; enabled?: boolean }[], bundled: Bundled[] = []): SkillsOverview {
  return {
    enabled: true,
    dir: '/data/skills',
    skills: skills.map((s) => ({
      name: s.name,
      description: `${s.name} description`,
      path: `/data/skills/${s.name}/SKILL.md`,
      enabled: s.enabled ?? true,
    })),
    bundled: bundled.map((b) => ({
      name: b.name,
      description: `${b.name} description`,
      enabled: b.enabled ?? false,
      shadowed: b.shadowed ?? false,
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
  setBundledSkillEnabled.mockResolvedValue();
  copyBundledSkill.mockResolvedValue();
  readBundledSkill.mockImplementation((n) => Promise.resolve(`---\nname: ${n}\n---\n**bold** <b>raw</b> of ${n}`));
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('SkillsView: bundled skills', () => {
  it('lists them with their description and their own switch, off', async () => {
    listSkills.mockResolvedValue(overview([], [{ name: 'inbox-triage' }]));
    await render();
    expect(q('bundled-row-inbox-triage')?.textContent).toContain('inbox-triage description');
    expect(q('bundled-toggle-inbox-triage')?.getAttribute('aria-checked')).toBe('false');
  });

  it('switches one on through its own preference, not the user skills one', async () => {
    listSkills.mockResolvedValue(overview([], [{ name: 'inbox-triage' }]));
    await render();
    listSkills.mockResolvedValue(overview([], [{ name: 'inbox-triage', enabled: true }]));
    await click(q('bundled-toggle-inbox-triage'));
    expect(setBundledSkillEnabled).toHaveBeenCalledWith('inbox-triage', true);
    expect(setSkillEnabled).not.toHaveBeenCalled();
    expect(q('bundled-toggle-inbox-triage')?.getAttribute('aria-checked')).toBe('true');
  });

  it('shows the full text read-only and as plain text', async () => {
    listSkills.mockResolvedValue(overview([], [{ name: 'inbox-triage' }]));
    await render();
    await click(q('bundled-row-inbox-triage'));
    const text = q('bundled-text');
    expect(text?.textContent).toContain('**bold** <b>raw</b> of inbox-triage');
    expect(text?.querySelector('b, strong')).toBeNull();
    expect(q('skill-editor')).toBeNull();
    expect(q('skill-save')).toBeNull();
    expect(q('skill-delete')).toBeNull();
  });

  it('opens a copy in the editor after Edit a copy', async () => {
    listSkills.mockResolvedValue(overview([], [{ name: 'inbox-triage' }]));
    await render();
    await click(q('bundled-row-inbox-triage'));
    listSkills.mockResolvedValue(overview([{ name: 'inbox-triage' }], [{ name: 'inbox-triage', shadowed: true }]));
    await click(q('bundled-copy'));
    expect(copyBundledSkill).toHaveBeenCalledWith('inbox-triage');
    expect(q('bundled-text')).toBeNull();
    expect((q('skill-editor') as HTMLTextAreaElement).value).toContain('body of inbox-triage');
  });

  it('shows why a copy was refused', async () => {
    listSkills.mockResolvedValue(overview([], [{ name: 'inbox-triage' }]));
    copyBundledSkill.mockRejectedValue({ code: 'skill_already_exists' });
    await render();
    await click(q('bundled-row-inbox-triage'));
    await click(q('bundled-copy'));
    expect(q('skills-error')?.textContent).toContain('translated:skill_already_exists');
    expect(q('bundled-text')).not.toBeNull();
  });

  it('a skill of the user with the same name replaces the switch with a note', async () => {
    listSkills.mockResolvedValue(
      overview([{ name: 'inbox-triage' }], [{ name: 'inbox-triage', enabled: true, shadowed: true }]),
    );
    await render();
    expect(q('bundled-toggle-inbox-triage')).toBeNull();
    expect(q('bundled-row-inbox-triage')?.textContent).toContain('settings:skills.view.bundledShadowed');
    await click(q('bundled-row-inbox-triage'));
    expect((q('bundled-copy') as HTMLButtonElement).disabled).toBe(true);
  });

  it('selecting a user skill with the same name opens the editor, not the bundled text', async () => {
    listSkills.mockResolvedValue(overview([{ name: 'inbox-triage' }], [{ name: 'inbox-triage', shadowed: true }]));
    await render();
    await click(q('bundled-row-inbox-triage'));
    expect(q('skill-editor')).toBeNull();
    await click(q('skill-row-inbox-triage'));
    expect(q('bundled-text')).toBeNull();
    expect(q('skill-editor')).not.toBeNull();
  });
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
