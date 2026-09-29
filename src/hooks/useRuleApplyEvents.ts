import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef } from 'react';
import * as api from '@/lib/api';
import { type AppErrorPayload, errorText, isAppErrorPayload } from '@/lib/errors';
import { type RuleApplyProgress, useAttachmentStore } from '@/stores/attachmentStore';
import { useLogStore } from '@/stores/logStore';

/** Payload of `attachment-rule-apply-finished`. */
export interface RuleApplyFinished {
  ruleId: string;
  runId: string;
  status: 'done' | 'failed' | 'cancelled';
  saved: number;
  error: AppErrorPayload | null;
}

export function isRuleApplyProgress(p: unknown): p is RuleApplyProgress {
  if (typeof p !== 'object' || p === null) return false;
  const o = p as Record<string, unknown>;
  return (
    typeof o.ruleId === 'string' &&
    typeof o.runId === 'string' &&
    typeof o.processed === 'number' &&
    typeof o.total === 'number' &&
    typeof o.saved === 'number'
  );
}

export function isRuleApplyFinished(p: unknown): p is RuleApplyFinished {
  if (typeof p !== 'object' || p === null) return false;
  const o = p as Record<string, unknown>;
  return (
    typeof o.ruleId === 'string' &&
    typeof o.runId === 'string' &&
    (o.status === 'done' || o.status === 'failed' || o.status === 'cancelled') &&
    typeof o.saved === 'number' &&
    (o.error === null || o.error === undefined || isAppErrorPayload(o.error))
  );
}

/**
 * App-wide listener for background rule scans. Mounted once (via
 * `useAttachments`), so a scan started from the rules modal still reports its
 * outcome — store, output panel, attachment list — after the modal closed.
 */
export function useRuleApplyEvents(onApplied: () => void) {
  const reportRuleApplyProgress = useAttachmentStore((s) => s.reportRuleApplyProgress);
  const finishRuleApply = useAttachmentStore((s) => s.finishRuleApply);
  const failRuleApply = useAttachmentStore((s) => s.failRuleApply);
  const dropRuleApply = useAttachmentStore((s) => s.dropRuleApply);
  const addLog = useLogStore((s) => s.addLog);
  // Latest callback without re-subscribing on every parent render.
  const onAppliedRef = useRef(onApplied);
  onAppliedRef.current = onApplied;

  useEffect(() => {
    const finished = async ({ ruleId, runId, status, saved, error }: RuleApplyFinished) => {
      if (status === 'cancelled') {
        // Superseded by a newer scan, or the rule was edited / deleted.
        dropRuleApply(ruleId, runId);
        addLog('debug', 'attachments', 'A rule scan was superseded');
        return;
      }
      if (status === 'failed') {
        failRuleApply(ruleId, runId);
        // The backend's English message stands in when no translation exists.
        const detail = error ? errorText(error) || error.message : 'unknown error';
        addLog('error', 'attachments', `Failed to apply rule retroactively: ${detail}`);
        return;
      }
      // `saved` is only what this run added; re-applying an edited rule adds
      // nothing new, and "0" read as if the rule had found nothing.
      const collected = await api.countAttachmentsForRule(ruleId).catch((err) => {
        addLog('error', 'attachments', `Failed to count rule attachments: ${errorText(err)}`);
        return undefined;
      });
      finishRuleApply(ruleId, runId, saved, collected);
      addLog('success', 'attachments', `Found ${saved} attachments from existing emails`);
      onAppliedRef.current();
    };

    const unlistenProgress = listen('attachment-rule-apply-progress', (event) => {
      if (isRuleApplyProgress(event.payload)) reportRuleApplyProgress(event.payload);
    });
    const unlistenFinished = listen('attachment-rule-apply-finished', (event) => {
      if (isRuleApplyFinished(event.payload)) void finished(event.payload);
      else addLog('debug', 'attachments', 'Ignored a malformed rule scan event');
    });
    return () => {
      void unlistenProgress.then((u) => u());
      void unlistenFinished.then((u) => u());
    };
  }, [reportRuleApplyProgress, finishRuleApply, failRuleApply, dropRuleApply, addLog]);
}
