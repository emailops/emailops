import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { InlineError } from '@/components/common/InlineError';
import { ToggleSwitch } from '@/components/common/ToggleSwitch';
import { approveAgentAction, getAgentOverview, rejectAgentAction, setAgentEnabled } from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { AgentOverview, AgentPanel } from '@/types';
import { AgentActionCard } from './AgentActionCard';
import { AgentFeedItem } from './AgentFeedItem';
import { AgentPanelDialog } from './AgentPanelDialog';
import { AgentReviewPane } from './AgentReviewPane';
import { AgentRulesDialog } from './AgentRulesDialog';
import { chronological, splitActions } from './agentFeed';

interface AgentViewProps {
  /** Switch to the inbox after an email or draft was opened from the feed. */
  onOpenEmail: () => void;
}

/**
 * The email agent: a chat-like feed of what the agent saw and did on new
 * mail and upcoming events, the user's stats panels on top, and on the right
 * what waits for the user (actions to approve, drafts to send or discard)
 * followed by the latest decided ones. A click on any action opens it in the
 * review pane, in place of the feed, so the user finishes the agent's work
 * without leaving the view. The view follows the backend live through the
 * `agent-updated` event.
 */
export function AgentView({ onOpenEmail }: AgentViewProps) {
  const { t } = useTranslation(['agent']);
  const addLog = useLogStore((s) => s.addLog);
  const [overview, setOverview] = useState<AgentOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [rulesOpen, setRulesOpen] = useState(false);
  // `undefined` = closed; `null` = a new panel; otherwise the panel being edited.
  const [panelEditing, setPanelEditing] = useState<AgentPanel | null | undefined>(undefined);
  // The action open in the review pane; `null` shows the feed.
  const [reviewing, setReviewing] = useState<string | null>(null);
  const feedEnd = useRef<HTMLDivElement | null>(null);
  const loadId = useRef(0);

  const fail = useCallback(
    (what: string, err: unknown) => {
      const msg = `${what}: ${errorText(err)}`;
      setError(msg);
      addLog('error', 'ai', msg);
    },
    [addLog],
  );

  const reload = useCallback(async () => {
    const id = ++loadId.current;
    try {
      const next = await getAgentOverview();
      if (id === loadId.current) setOverview(next);
    } catch (err) {
      if (id === loadId.current) fail(t('agent:error.load'), err);
    }
  }, [fail, t]);

  useEffect(() => {
    void reload();
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen('agent-updated', () => void reload()).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [reload]);

  const feed = useMemo(() => chronological(overview?.feed ?? []), [overview]);
  const { pending, recent } = useMemo(() => splitActions(overview?.actions ?? []), [overview]);
  const runsById = useMemo(() => new Map((overview?.feed ?? []).map((r) => [r.id, r])), [overview]);
  const reviewed = useMemo(
    () => (reviewing ? (overview?.actions.find((a) => a.id === reviewing) ?? findInFeed(overview, reviewing)) : null),
    [overview, reviewing],
  );

  // Keep the newest message in view, like a chat.
  useEffect(() => {
    if (feed.length > 0) feedEnd.current?.scrollIntoView?.({ block: 'end' });
  }, [feed.length]);

  const decide = async (id: string, approve: boolean) => {
    setBusyAction(id);
    setError(null);
    try {
      await (approve ? approveAgentAction(id) : rejectAgentAction(id));
      await reload();
    } catch (err) {
      fail(t('agent:error.action'), err);
    } finally {
      setBusyAction(null);
    }
  };

  const toggle = async (enabled: boolean) => {
    setError(null);
    try {
      await setAgentEnabled(enabled);
      await reload();
    } catch (err) {
      fail(t('agent:error.save'), err);
    }
  };

  return (
    <div className="flex flex-1 min-h-0 min-w-0 overflow-hidden bg-[#1e1e1e]" data-testid="agent-view">
      <section className="flex flex-1 min-w-0 flex-col min-h-0">
        <header className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-gray-700 px-5 py-3">
          <div className="min-w-[12rem] flex-1">
            <h2 className="text-base font-semibold text-gray-100">{t('agent:title')}</h2>
            <p className="truncate text-xs text-gray-400">{t('agent:subtitle')}</p>
          </div>
          <button
            type="button"
            data-testid="agent-rules-button"
            onClick={() => setRulesOpen(true)}
            className="whitespace-nowrap rounded bg-gray-700 px-3 py-1.5 text-sm text-gray-200 hover:bg-gray-600"
          >
            {t('agent:rules.button', { count: overview?.rules.length ?? 0 })}
          </button>
          <ToggleSwitch
            checked={overview?.enabled ?? false}
            disabled={!overview}
            onChange={(next) => void toggle(next)}
            label={<span className="whitespace-nowrap text-sm text-gray-200">{t('agent:toggle')}</span>}
          />
        </header>

        <div className="flex gap-3 overflow-x-auto border-b border-gray-700 px-5 py-3">
          {overview?.panels.map((panel) => (
            <button
              type="button"
              key={panel.id}
              data-testid={`agent-panel-${panel.id}`}
              onClick={() => setPanelEditing(panel)}
              title={panel.prompt}
              className="min-w-[150px] rounded-lg border border-gray-700 bg-gray-800/70 px-4 py-2 text-left hover:border-gray-500"
            >
              <span className="block text-2xl font-semibold text-gray-100">{panel.count}</span>
              <span className="block truncate text-xs text-gray-300">{panel.title}</span>
              <span className="block text-[10px] uppercase text-gray-500">{t(`agent:panels.${panel.window}`)}</span>
            </button>
          ))}
          <button
            type="button"
            data-testid="agent-panel-new"
            onClick={() => setPanelEditing(null)}
            className="min-w-[120px] rounded-lg border border-dashed border-gray-600 px-4 py-2 text-sm text-gray-400 hover:border-gray-400 hover:text-gray-200"
          >
            + {t('agent:panels.new')}
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-5 py-4">
          {error && (
            <div data-testid="agent-error" className="mb-3">
              <InlineError message={error} />
            </div>
          )}
          {overview && !overview.enabled && (
            <p
              data-testid="agent-off-notice"
              className="mb-4 rounded-lg border border-amber-800 bg-amber-900/20 px-3 py-2 text-sm text-amber-200"
            >
              {t('agent:off')}
            </p>
          )}
          {reviewed ? (
            <AgentReviewPane
              key={reviewed.id}
              action={reviewed}
              run={runsById.get(reviewed.runId)}
              busy={busyAction === reviewed.id}
              onBack={() => setReviewing(null)}
              onApprove={(id) => void decide(id, true)}
              onReject={(id) => void decide(id, false)}
              onChanged={() => void reload()}
              onOpenEmail={onOpenEmail}
            />
          ) : overview && feed.length === 0 ? (
            <div data-testid="agent-feed-empty" className="mx-auto mt-10 max-w-md text-center text-sm text-gray-400">
              <p>{t('agent:feed.empty')}</p>
              <button
                type="button"
                data-testid="agent-feed-create-rule"
                onClick={() => setRulesOpen(true)}
                className="mt-4 rounded bg-primary-600 px-3 py-1.5 text-sm text-white hover:bg-primary-500"
              >
                {t('agent:feed.emptyCta')}
              </button>
            </div>
          ) : (
            <ul className="space-y-4">
              {feed.map((run) => (
                <AgentFeedItem key={run.id} run={run} onOpenEmail={onOpenEmail} onSelectAction={setReviewing} />
              ))}
            </ul>
          )}
          {!reviewed && <div ref={feedEnd} />}
        </div>
      </section>

      <aside className="flex w-72 flex-shrink-0 flex-col min-h-0 border-l border-gray-700">
        <h3 className="border-b border-gray-700 px-4 py-3 text-sm font-semibold text-gray-200">
          {t('agent:actions.title')}
        </h3>
        <div className="flex-1 space-y-4 overflow-y-auto px-3 py-3">
          <div>
            <p className="mb-2 px-1 text-xs font-medium uppercase text-amber-300">
              {t('agent:actions.pending', { count: pending.length })}
            </p>
            {pending.length === 0 && <p className="px-1 text-xs text-gray-500">{t('agent:actions.none')}</p>}
            <ul className="space-y-2">
              {pending.map((a) => (
                <AgentActionCard
                  key={a.id}
                  action={a}
                  selected={a.id === reviewing}
                  busy={busyAction === a.id}
                  onSelect={setReviewing}
                  onApprove={(id) => void decide(id, true)}
                  onReject={(id) => void decide(id, false)}
                />
              ))}
            </ul>
          </div>
          {recent.length > 0 && (
            <div>
              <p className="mb-2 px-1 text-xs font-medium uppercase text-gray-400">{t('agent:actions.recent')}</p>
              <ul className="space-y-2">
                {recent.map((a) => (
                  <AgentActionCard
                    key={a.id}
                    action={a}
                    selected={a.id === reviewing}
                    busy={false}
                    onSelect={setReviewing}
                    onApprove={(id) => void decide(id, true)}
                    onReject={(id) => void decide(id, false)}
                  />
                ))}
              </ul>
            </div>
          )}
        </div>
      </aside>

      <AgentRulesDialog
        open={rulesOpen}
        rules={overview?.rules ?? []}
        onClose={() => setRulesOpen(false)}
        onChanged={() => void reload()}
      />
      {panelEditing !== undefined && (
        <AgentPanelDialog
          panel={panelEditing}
          onClose={() => setPanelEditing(undefined)}
          onChanged={() => void reload()}
        />
      )}
    </div>
  );
}

/** An action of a run in the feed that the side panel no longer lists. */
function findInFeed(overview: AgentOverview | null, id: string) {
  for (const run of overview?.feed ?? []) {
    const found = run.actions.find((a) => a.id === id);
    if (found) return found;
  }
  return null;
}
