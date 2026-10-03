import type { AgentAction, AgentRun } from '@/types';

/** The feed as a chat reads it: oldest first, newest at the bottom. */
export function chronological(feed: AgentRun[]): AgentRun[] {
  return [...feed].sort((a, b) => a.createdAt - b.createdAt);
}

/** The side panel's two lists: actions waiting for review, and the decided ones. */
export function splitActions(actions: AgentAction[]): { pending: AgentAction[]; recent: AgentAction[] } {
  return {
    pending: actions.filter((a) => a.status === 'pending'),
    recent: actions.filter((a) => a.status !== 'pending'),
  };
}
