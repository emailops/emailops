import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { createSkill, listSkills, readSkill, type SkillsOverview, saveSkill, setSkillEnabled } from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';

/**
 * The Skills view: every skill in `<data dir>/skills/` on the left, each with
 * its own on/off switch, and the selected skill's `SKILL.md` in an editor on
 * the right. The file on disk stays the source of truth — Save writes it back
 * (the backend refuses text the chat could not load) and the folder can still
 * be edited in any other editor.
 */
export function SkillsView() {
  const { t } = useTranslation(['settings']);
  const addLog = useLogStore((s) => s.addLog);
  const [overview, setOverview] = useState<SkillsOverview | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  // Text as last read from / written to disk, and unsaved edits, per skill:
  // switching skills never throws an edit away.
  const [saved, setSaved] = useState<Record<string, string>>({});
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState('');

  const fail = useCallback(
    (what: string, err: unknown) => {
      const msg = errorText(err);
      setError(msg);
      addLog('error', 'ai', `${what}: ${msg}`);
    },
    [addLog],
  );

  const open = useCallback(
    async (name: string) => {
      setSelected(name);
      setError(null);
      setStatus(null);
      try {
        const text = await readSkill(name);
        setSaved((s) => ({ ...s, [name]: text }));
      } catch (err) {
        fail(`Failed to read skill ${name}`, err);
      }
    },
    [fail],
  );

  const reload = useCallback(async () => {
    try {
      const o = await listSkills();
      setOverview(o);
      return o;
    } catch (err) {
      fail('Failed to list skills', err);
      return null;
    }
  }, [fail]);

  useEffect(() => {
    void (async () => {
      const o = await reload();
      const first = o?.skills[0]?.name;
      if (first) await open(first);
    })();
  }, [reload, open]);

  const handleToggle = async (name: string, enabled: boolean) => {
    setError(null);
    try {
      await setSkillEnabled(name, enabled);
      addLog('info', 'ai', `Skill ${name} ${enabled ? 'enabled' : 'disabled'}`);
      await reload();
    } catch (err) {
      fail(`Failed to switch skill ${name}`, err);
    }
  };

  const text = selected ? (drafts[selected] ?? saved[selected] ?? '') : '';
  const dirty = selected !== null && drafts[selected] !== undefined && drafts[selected] !== saved[selected];

  const handleSave = async () => {
    if (!selected) return;
    // What goes to disk, and what disk held when the editor opened it: the
    // backend refuses the save if someone edited the file in the meantime.
    const sent = text;
    const base = saved[selected] ?? '';
    setError(null);
    setStatus(null);
    try {
      const name = await saveSkill(selected, sent, base);
      setSaved(({ [selected]: _, ...rest }) => ({ ...rest, [name]: sent }));
      // Keep anything typed while the save was in flight.
      setDrafts(({ [selected]: current, ...rest }) =>
        current !== undefined && current !== sent ? { ...rest, [name]: current } : rest,
      );
      if (name !== selected) {
        setSelected(name);
        addLog('success', 'ai', `Saved skill ${selected} as ${name}`);
      } else {
        addLog('success', 'ai', `Saved skill ${name}`);
      }
      setStatus(t('settings:skills.view.saved'));
      await reload();
    } catch (err) {
      fail(`Failed to save skill ${selected}`, err);
    }
  };

  const handleReloadFromDisk = async () => {
    if (!selected) return;
    const name = selected;
    setDrafts(({ [name]: _, ...rest }) => rest);
    await open(name);
  };

  const handleCreate = async () => {
    const name = newName.trim();
    if (!name) return;
    setError(null);
    try {
      await createSkill(name);
      addLog('success', 'ai', `Created skill ${name}`);
      setCreating(false);
      setNewName('');
      await reload();
      await open(name);
    } catch (err) {
      fail(`Failed to create skill ${name}`, err);
    }
  };

  return (
    <div className="flex flex-1 min-h-0 min-w-0 overflow-hidden bg-[#1e1e1e]">
      <aside className="w-80 flex-shrink-0 border-r border-gray-700 flex flex-col min-h-0">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-700">
          <h2 className="text-sm font-semibold text-gray-200 flex-1">{t('settings:skills.view.title')}</h2>
          <span className="px-1.5 py-0.5 rounded bg-amber-900/40 text-amber-300 text-[10px] font-semibold uppercase">
            {t('settings:dialog.experimental')}
          </span>
          <button
            type="button"
            data-testid="skill-new"
            onClick={() => setCreating((c) => !c)}
            className="px-2 py-1 text-xs rounded bg-primary-600 hover:bg-primary-500 text-white"
          >
            {t('settings:skills.view.new')}
          </button>
        </div>
        {creating && (
          <form
            className="flex gap-2 px-4 py-2 border-b border-gray-700"
            onSubmit={(e) => {
              e.preventDefault();
              void handleCreate();
            }}
          >
            <input
              data-testid="skill-new-name"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder={t('settings:skills.view.newPlaceholder')}
              className="flex-1 min-w-0 px-2 py-1 text-xs rounded bg-gray-800 border border-gray-600 text-gray-200"
            />
            <button
              type="submit"
              data-testid="skill-create"
              className="px-2 py-1 text-xs rounded bg-gray-700 hover:bg-gray-600 text-gray-200"
            >
              {t('settings:skills.view.create')}
            </button>
          </form>
        )}
        <ul className="flex-1 overflow-y-auto py-1">
          {overview && overview.skills.length === 0 && (
            <li className="px-4 py-3 text-xs text-gray-500">{t('settings:skills.view.empty')}</li>
          )}
          {overview?.skills.map((s) => (
            <li
              key={s.name}
              className={`flex items-start gap-2 px-4 py-2 ${selected === s.name ? 'bg-gray-700/60' : 'hover:bg-gray-800'}`}
            >
              <button
                type="button"
                data-testid={`skill-row-${s.name}`}
                onClick={() => void open(s.name)}
                className="flex-1 min-w-0 text-left"
              >
                <span className={`block text-sm truncate ${s.enabled ? 'text-gray-200' : 'text-gray-500'}`}>
                  {s.name}
                  {drafts[s.name] !== undefined && drafts[s.name] !== saved[s.name] && (
                    <span className="ml-1 text-amber-400">•</span>
                  )}
                </span>
                <span className="block text-xs text-gray-500 line-clamp-2">{s.description}</span>
              </button>
              <button
                type="button"
                role="switch"
                aria-checked={s.enabled}
                aria-label={t('settings:skills.view.toggle', { name: s.name })}
                data-testid={`skill-toggle-${s.name}`}
                onClick={() => void handleToggle(s.name, !s.enabled)}
                className={`relative mt-0.5 inline-flex h-5 w-9 flex-shrink-0 rounded-full border-2 border-transparent transition-colors ${
                  s.enabled ? 'bg-primary-600' : 'bg-gray-600'
                }`}
              >
                <span
                  className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition ${
                    s.enabled ? 'translate-x-4' : 'translate-x-0'
                  }`}
                />
              </button>
            </li>
          ))}
        </ul>
        {overview && overview.errors.length > 0 && (
          <div className="border-t border-gray-700 px-4 py-2 space-y-1 max-h-40 overflow-y-auto">
            <p className="text-xs font-medium text-red-300">{t('settings:skills.errorsTitle')}</p>
            {overview.errors.map((e) => (
              <p key={e.path} className="text-xs text-red-300/80 break-all">
                <code>{e.path}</code>: {e.message}
              </p>
            ))}
          </div>
        )}
      </aside>

      <section className="flex-1 min-w-0 flex flex-col min-h-0">
        {error && (
          <div
            data-testid="skills-error"
            className="m-3 mb-0 p-3 bg-red-900/30 border border-red-800 rounded text-red-300 text-sm"
          >
            {error}
            {selected && (
              <button
                type="button"
                data-testid="skill-reload"
                onClick={() => void handleReloadFromDisk()}
                className="ml-3 px-2 py-0.5 text-xs rounded bg-gray-700 hover:bg-gray-600 text-gray-200"
              >
                {t('settings:skills.view.reloadFromDisk')}
              </button>
            )}
          </div>
        )}
        {selected ? (
          <>
            <div className="flex items-center gap-3 px-4 py-2 border-b border-gray-700">
              <code className="flex-1 min-w-0 truncate text-xs text-gray-400">{selected}/SKILL.md</code>
              {status && !dirty && <span className="text-xs text-green-400">{status}</span>}
              {dirty && <span className="text-xs text-amber-400">{t('settings:skills.view.unsaved')}</span>}
              <button
                type="button"
                data-testid="skill-save"
                onClick={() => void handleSave()}
                disabled={!dirty}
                className="px-3 py-1 text-xs rounded bg-primary-600 hover:bg-primary-500 disabled:opacity-40 text-white"
              >
                {t('settings:skills.view.save')}
              </button>
            </div>
            <textarea
              data-testid="skill-editor"
              value={text}
              onChange={(e) => setDrafts((d) => ({ ...d, [selected]: e.target.value }))}
              spellCheck={false}
              className="flex-1 min-h-0 w-full resize-none bg-[#1a1a1a] text-gray-200 font-mono text-[13px] leading-5 p-4 focus:outline-none"
            />
            <p className="px-4 py-2 text-[11px] text-gray-500 border-t border-gray-700">
              {t('settings:skills.view.hint')}
            </p>
          </>
        ) : (
          <div className="flex-1 flex items-center justify-center text-sm text-gray-500">
            {t('settings:skills.view.pick')}
          </div>
        )}
      </section>
    </div>
  );
}
