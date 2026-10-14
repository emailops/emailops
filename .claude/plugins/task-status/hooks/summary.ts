// Pure helpers for the task-status mod; register.ts does the I/O.

export type Summary = {
  counts: Record<string, number>;
  needs_you: string[];
  text: string;
};

/** The summary `task_board.py summary --json` printed, or null when it is not one. */
export function parseSummary(stdout: string): Summary | null {
  try {
    const value = JSON.parse(stdout);
    if (typeof value?.text === 'string' && Array.isArray(value?.needs_you)) {
      return value as Summary;
    }
  } catch {
    // Not JSON: the caller shows the board as unavailable.
  }
  return null;
}

/** Task ids that wait for the user now and did not on the previous poll. */
export function newlyWaiting(previous: readonly string[] | null, current: readonly string[]): string[] {
  if (previous === null) return []; // the first poll after start: nothing is news yet
  return current.filter((id) => !previous.includes(id));
}

/** The main checkout of the repo whose git common dir is given, or null. */
export function mainCheckout(gitCommonDir: string): string | null {
  const dir = gitCommonDir.trim().replace(/\/+$/, '');
  return dir.endsWith('/.git') ? dir.slice(0, -'/.git'.length) : null;
}
