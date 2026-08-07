import { format } from 'date-fns';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { EmailPreviewById } from '@/components/shared/EmailPreviewById';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useMemoryStore } from '@/stores/memoryStore';
import type { PendingTask, ThreadState } from '@/types';

interface TasksPanelProps {
  accountId: string | null;
}

const ONE_DAY = 86_400;

type Section = 'overdue' | 'today' | 'upcoming' | 'noDate';

function sectionFor(task: PendingTask, nowSec: number): Section {
  if (task.dueAt == null) return 'noDate';
  if (task.dueAt < nowSec) return 'overdue';
  if (task.dueAt < nowSec + ONE_DAY) return 'today';
  return 'upcoming';
}

function formatDue(ts: number | null): string {
  if (ts == null) return 'No due date';
  return format(new Date(ts * 1000), 'MMM d, yyyy');
}

function formatLastTouched(ts: number): string {
  return format(new Date(ts * 1000), 'MMM d');
}

export function TasksPanel({ accountId }: TasksPanelProps) {
  const { t } = useTranslation(['tasks', 'common']);
  const {
    tasks,
    openThreads,
    counts,
    isLoadingTasks,
    isLoadingThreads,
    error,
    loadForAccount,
    setTaskStatus,
    createTask,
  } = useMemoryStore();
  const [pendingStatusId, setPendingStatusId] = useState<string | null>(null);
  const [newTitle, setNewTitle] = useState('');
  const [isCreating, setIsCreating] = useState(false);
  // Which entity is driving the right-side email preview: either a pending
  // task (→ task.sourceEmailId) or a waiting-on-reply thread (→ latest email
  // in the thread, resolved via get_thread).
  const [selection, setSelection] = useState<
    | { kind: 'task'; taskId: string }
    | { kind: 'thread'; threadId: string; resolvedEmailId: string | null; isLoading: boolean; error: string | null }
    | null
  >(null);
  const [companyFilter, setCompanyFilter] = useState<string | null>(null);

  useEffect(() => {
    if (accountId) {
      void loadForAccount(accountId);
    }
  }, [accountId, loadForAccount]);

  // Drop the selection if the entity disappears (task marked done, thread
  // closed, account switched).
  useEffect(() => {
    if (selection?.kind === 'task' && !tasks.some((t) => t.id === selection.taskId)) {
      setSelection(null);
    }
    if (selection?.kind === 'thread' && !openThreads.some((t) => t.threadId === selection.threadId)) {
      setSelection(null);
    }
  }, [tasks, openThreads, selection]);

  const selectedTaskId = selection?.kind === 'task' ? selection.taskId : null;

  // Resolve a thread selection to its latest email id. Runs once per
  // thread selection; ignores stale responses if the user moves on.
  useEffect(() => {
    if (!accountId || selection?.kind !== 'thread' || selection.resolvedEmailId || selection.error) {
      return;
    }
    const threadId = selection.threadId;
    let cancelled = false;
    (async () => {
      try {
        const emails = await api.getThread(accountId, threadId);
        if (cancelled) return;
        const latest = emails.length > 0 ? emails[emails.length - 1].id : null;
        setSelection((prev) =>
          prev?.kind === 'thread' && prev.threadId === threadId
            ? { ...prev, resolvedEmailId: latest, isLoading: false }
            : prev,
        );
      } catch (e) {
        if (cancelled) return;
        const message = errorText(e);
        setSelection((prev) =>
          prev?.kind === 'thread' && prev.threadId === threadId ? { ...prev, error: message, isLoading: false } : prev,
        );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [accountId, selection]);

  const nowSec = Math.floor(Date.now() / 1000);

  // Rank companies by task count so the chip bar is stable and predictable.
  const companyChips = useMemo(() => {
    const counts = new Map<string, number>();
    for (const t of tasks) {
      if (!t.company) continue;
      counts.set(t.company, (counts.get(t.company) ?? 0) + 1);
    }
    return [...counts.entries()]
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .map(([company, count]) => ({ company, count }));
  }, [tasks]);

  useEffect(() => {
    if (companyFilter && !companyChips.some((c) => c.company === companyFilter)) {
      setCompanyFilter(null);
    }
  }, [companyFilter, companyChips]);

  const visibleTasks = useMemo(
    () => (companyFilter ? tasks.filter((t) => t.company === companyFilter) : tasks),
    [tasks, companyFilter],
  );

  const grouped = useMemo(() => {
    const buckets: Record<Section, PendingTask[]> = {
      overdue: [],
      today: [],
      upcoming: [],
      noDate: [],
    };
    for (const t of visibleTasks) {
      buckets[sectionFor(t, nowSec)].push(t);
    }
    return buckets;
  }, [visibleTasks, nowSec]);

  const selectedTask = useMemo(
    () => (selectedTaskId ? (tasks.find((t) => t.id === selectedTaskId) ?? null) : null),
    [tasks, selectedTaskId],
  );

  const previewEmailId = useMemo<string | null>(() => {
    if (selection?.kind === 'task') return selectedTask?.sourceEmailId ?? null;
    if (selection?.kind === 'thread') return selection.resolvedEmailId;
    return null;
  }, [selection, selectedTask]);
  const previewHasSelection = selection !== null;
  const previewMissingMessage =
    selection?.kind === 'task'
      ? t('tasks:panel.previewMissingTask')
      : selection?.kind === 'thread' && selection.error
        ? selection.error
        : t('tasks:panel.previewMissingThread');

  const handleStatus = async (task: PendingTask, status: string) => {
    setPendingStatusId(task.id);
    try {
      await setTaskStatus(task.id, status);
    } catch {
      // Error surfaced via store.error; nothing extra here.
    } finally {
      setPendingStatusId(null);
    }
  };

  const handleCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    const title = newTitle.trim();
    if (!title || !accountId) return;
    setIsCreating(true);
    try {
      await createTask({ accountId, title });
      setNewTitle('');
    } finally {
      setIsCreating(false);
    }
  };

  if (!accountId) {
    return (
      <div className="flex flex-col flex-1 items-center justify-center text-sm text-gray-500 bg-white dark:text-gray-400 dark:bg-surface">
        {t('tasks:panel.selectAccount')}
      </div>
    );
  }

  return (
    <div className="flex flex-1 overflow-hidden bg-white dark:bg-surface">
      {/* Left: task list */}
      <div className="flex flex-col w-[480px] flex-shrink-0 border-r border-gray-200 overflow-hidden dark:border-gray-700">
        <div className="px-6 py-4 border-b border-gray-200 flex-shrink-0 flex items-center justify-between dark:border-gray-700">
          <div>
            <h1 className="text-xl font-semibold text-gray-900 dark:text-gray-100">{t('tasks:title')}</h1>
            <p className="text-xs text-gray-500 mt-0.5 dark:text-gray-400">
              {t('tasks:panel.openCount', { count: counts.totalOpen })}
              {counts.overdue > 0 && t('tasks:panel.overdueSuffix', { count: counts.overdue })}
              {counts.dueToday > 0 && t('tasks:panel.dueTodaySuffix', { count: counts.dueToday })}
              {counts.awaitingThem > 0 && t('tasks:panel.waitingSuffix', { count: counts.awaitingThem })}
            </p>
          </div>
          <button
            type="button"
            onClick={() => {
              void useMemoryStore.getState().refreshTasks();
              void useMemoryStore.getState().refreshOpenThreads();
              void useMemoryStore.getState().refreshCounts();
            }}
            className="text-xs text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-300"
          >
            {t('common:actions.refresh')}
          </button>
        </div>

        {error && (
          <div className="px-6 py-2 bg-red-50 border-b border-red-200 text-sm text-red-700 dark:bg-red-900/20 dark:border-red-800 dark:text-red-300">
            {error}
          </div>
        )}

        <div className="flex-1 overflow-y-auto p-6 space-y-6">
          <form onSubmit={handleCreate} className="flex gap-2">
            <input
              type="text"
              value={newTitle}
              onChange={(e) => setNewTitle(e.target.value)}
              placeholder={t('tasks:panel.addPlaceholder')}
              className="flex-1 px-3 py-2 text-sm border border-gray-300 rounded-md focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-gray-600"
              disabled={isCreating}
            />
            <button
              type="submit"
              disabled={!newTitle.trim() || isCreating}
              className="px-4 py-2 text-sm font-medium text-white bg-primary-600 rounded-md hover:bg-primary-700 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {t('tasks:panel.addButton')}
            </button>
          </form>

          {companyChips.length > 0 && (
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-xs text-gray-500 mr-1 dark:text-gray-400">{t('tasks:panel.companyLabel')}</span>
              <CompanyChip
                label={t('tasks:panel.allChip')}
                count={tasks.length}
                active={companyFilter === null}
                onClick={() => setCompanyFilter(null)}
              />
              {companyChips.map(({ company, count }) => (
                <CompanyChip
                  key={company}
                  label={company}
                  count={count}
                  active={companyFilter === company}
                  onClick={() => setCompanyFilter(companyFilter === company ? null : company)}
                />
              ))}
            </div>
          )}

          {isLoadingTasks ? (
            <div className="text-sm text-gray-500 dark:text-gray-400">{t('tasks:panel.loadingTasks')}</div>
          ) : visibleTasks.length === 0 ? (
            <div className="text-sm text-gray-500 italic dark:text-gray-400">
              {tasks.length === 0
                ? t('tasks:panel.noOpenTasks')
                : t('tasks:panel.noTasksForFilter', { filter: companyFilter ?? t('tasks:panel.thisFilter') })}
            </div>
          ) : (
            <>
              <TaskSection
                title={t('tasks:panel.sections.overdue')}
                accent="text-red-600 dark:text-red-400"
                tasks={grouped.overdue}
                pendingStatusId={pendingStatusId}
                selectedTaskId={selectedTaskId}
                onSelect={(taskId) => setSelection({ kind: 'task', taskId })}
                onStatus={handleStatus}
              />
              <TaskSection
                title={t('tasks:panel.sections.dueToday')}
                accent="text-orange-600 dark:text-orange-400"
                tasks={grouped.today}
                pendingStatusId={pendingStatusId}
                selectedTaskId={selectedTaskId}
                onSelect={(taskId) => setSelection({ kind: 'task', taskId })}
                onStatus={handleStatus}
              />
              <TaskSection
                title={t('tasks:panel.sections.upcoming')}
                accent="text-gray-700 dark:text-gray-300"
                tasks={grouped.upcoming}
                pendingStatusId={pendingStatusId}
                selectedTaskId={selectedTaskId}
                onSelect={(taskId) => setSelection({ kind: 'task', taskId })}
                onStatus={handleStatus}
              />
              <TaskSection
                title={t('tasks:panel.sections.noDate')}
                accent="text-gray-500 dark:text-gray-400"
                tasks={grouped.noDate}
                pendingStatusId={pendingStatusId}
                selectedTaskId={selectedTaskId}
                onSelect={(taskId) => setSelection({ kind: 'task', taskId })}
                onStatus={handleStatus}
              />
            </>
          )}

          <WaitingOnThemSection
            isLoading={isLoadingThreads}
            threads={openThreads}
            selectedThreadId={selection?.kind === 'thread' ? selection.threadId : null}
            onSelect={(threadId) =>
              setSelection({
                kind: 'thread',
                threadId,
                resolvedEmailId: null,
                isLoading: true,
                error: null,
              })
            }
          />
        </div>
      </div>

      {/* Right: originating email preview */}
      <div className="flex-1 min-w-0 overflow-hidden">
        <EmailPreviewById
          accountId={accountId}
          emailId={previewEmailId}
          hasSelection={previewHasSelection}
          emptyMessage={t('tasks:panel.previewEmpty')}
          missingSourceMessage={previewMissingMessage}
        />
      </div>
    </div>
  );
}

interface TaskSectionProps {
  title: string;
  accent: string;
  tasks: PendingTask[];
  pendingStatusId: string | null;
  selectedTaskId: string | null;
  onSelect: (taskId: string) => void;
  onStatus: (task: PendingTask, status: string) => void;
}

function TaskSection({ title, accent, tasks, pendingStatusId, selectedTaskId, onSelect, onStatus }: TaskSectionProps) {
  const { t } = useTranslation(['tasks']);
  if (tasks.length === 0) return null;
  return (
    <section>
      <h2 className={`text-xs font-semibold uppercase tracking-wider mb-2 ${accent}`}>
        {t('tasks:panel.sections.count', { title, n: tasks.length })}
      </h2>
      <ul className="space-y-2">
        {tasks.map((task) => {
          const selected = selectedTaskId === task.id;
          return (
            <li
              key={task.id}
              onClick={() => onSelect(task.id)}
              className={`flex items-start gap-3 p-3 border rounded-md cursor-pointer transition-colors ${
                selected
                  ? 'border-primary-400 bg-primary-50 dark:bg-primary-900/20'
                  : 'border-gray-200 hover:bg-gray-50 dark:border-gray-700 dark:hover:bg-surface-raised'
              }`}
            >
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onStatus(task, 'done');
                }}
                disabled={pendingStatusId === task.id}
                title={t('tasks:panel.markDoneTitle')}
                className="mt-0.5 w-5 h-5 flex-shrink-0 border-2 border-gray-300 rounded hover:border-primary-500 disabled:opacity-50 dark:border-gray-600"
              />
              <div className="flex-1 min-w-0">
                <div className="text-sm text-gray-900 truncate dark:text-gray-100">{task.title}</div>
                {task.detail && (
                  <div className="text-xs text-gray-500 mt-0.5 line-clamp-2 dark:text-gray-400">{task.detail}</div>
                )}
                <div className="flex items-center gap-2 mt-1 text-xs text-gray-500 dark:text-gray-400">
                  <span>{formatDue(task.dueAt)}</span>
                  {task.company && (
                    <span
                      className="px-1.5 py-0.5 rounded bg-indigo-100 text-indigo-800 uppercase text-[10px] font-semibold dark:bg-indigo-900/30 dark:text-indigo-300"
                      title={t('tasks:panel.companyTitle')}
                    >
                      {task.company}
                    </span>
                  )}
                  {task.priority === 'high' && (
                    <span className="px-1.5 py-0.5 rounded bg-red-100 text-red-700 uppercase text-[10px] font-semibold dark:bg-red-900/30 dark:text-red-300">
                      {t('tasks:panel.high')}
                    </span>
                  )}
                  {task.source !== 'user' && (
                    <span className="px-1.5 py-0.5 rounded bg-gray-100 text-gray-600 uppercase text-[10px] dark:bg-surface-hover dark:text-gray-400">
                      {task.source}
                    </span>
                  )}
                </div>
              </div>
              <div className="flex flex-col gap-1">
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    onStatus(task, 'snoozed');
                  }}
                  disabled={pendingStatusId === task.id}
                  className="text-xs text-gray-500 hover:text-gray-700 px-2 py-1 disabled:opacity-50 dark:text-gray-400 dark:hover:text-gray-300"
                >
                  {t('tasks:panel.snooze')}
                </button>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    onStatus(task, 'dismissed');
                  }}
                  disabled={pendingStatusId === task.id}
                  className="text-xs text-gray-500 hover:text-gray-700 px-2 py-1 disabled:opacity-50 dark:text-gray-400 dark:hover:text-gray-300"
                >
                  {t('tasks:panel.dismiss')}
                </button>
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

interface CompanyChipProps {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}

function CompanyChip({ label, count, active, onClick }: CompanyChipProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs transition-colors ${
        active
          ? 'bg-primary-600 text-white'
          : 'bg-gray-100 text-gray-700 hover:bg-gray-200 dark:bg-surface-hover dark:text-gray-300 dark:hover:bg-gray-700'
      }`}
    >
      <span className="font-medium">{label}</span>
      <span className={active ? 'text-primary-100' : 'text-gray-500 dark:text-gray-400'}>{count}</span>
    </button>
  );
}

interface WaitingSectionProps {
  isLoading: boolean;
  threads: ThreadState[];
  selectedThreadId: string | null;
  onSelect: (threadId: string) => void;
}

function WaitingOnThemSection({ isLoading, threads, selectedThreadId, onSelect }: WaitingSectionProps) {
  const { t } = useTranslation(['tasks']);
  if (isLoading) {
    return <div className="text-sm text-gray-500 dark:text-gray-400">{t('tasks:panel.loadingThreads')}</div>;
  }
  if (threads.length === 0) return null;
  return (
    <section>
      <h2 className="text-xs font-semibold uppercase tracking-wider mb-2 text-gray-700 dark:text-gray-300">
        {t('tasks:panel.waitingTitle', { n: threads.length })}
      </h2>
      <ul className="space-y-2">
        {threads.map((thread) => {
          const selected = selectedThreadId === thread.threadId;
          return (
            <li
              key={thread.threadId}
              onClick={() => onSelect(thread.threadId)}
              className={`p-3 border rounded-md cursor-pointer transition-colors ${
                selected
                  ? 'border-primary-400 bg-primary-50 dark:bg-primary-900/20'
                  : 'border-gray-200 hover:bg-gray-50 dark:border-gray-700 dark:hover:bg-surface-raised'
              }`}
            >
              <div className="text-sm text-gray-900 truncate dark:text-gray-100">
                {thread.summary ?? thread.threadId}
              </div>
              {thread.commitment && (
                <div className="text-xs text-gray-500 mt-0.5 dark:text-gray-400">{thread.commitment}</div>
              )}
              <div className="text-xs text-gray-500 mt-1 dark:text-gray-400">
                {t('tasks:panel.lastTouched', { when: formatLastTouched(thread.lastTouchedAt) })}
                {thread.deadlineAt != null && t('tasks:panel.deadlineSuffix', { date: formatDue(thread.deadlineAt) })}
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
