import type { Register } from 'claude-code';

import { mainCheckout, newlyWaiting, parseSummary } from './summary';

const POLL_MS = 20_000;

export const register: Register = (on) => {
  on('session.start', async ($, e, next) => {
    const result = await next(e);
    const git = await $.process.run(['git', 'rev-parse', '--path-format=absolute', '--git-common-dir']);
    const root = git.exitCode === 0 ? mainCheckout(git.stdout) : null;
    const script = root && `${root}/scripts/task_board.py`;
    if (
      !script ||
      !(await $.fs.stat(script).then(
        (s) => s.kind === 'file',
        () => false,
      ))
    ) {
      return result; // not a checkout with the task board: stay out of the way
    }

    let waiting: string[] | null = null;
    let failed = false;
    const poll = async () => {
      const ran = await $.process
        .run(['python3', script, 'summary', '--json'], { cwd: root, timeoutMs: 15_000 })
        .catch((err: unknown) => ({ exitCode: 1, stdout: '', stderr: String(err) }));
      const summary = ran.exitCode === 0 ? parseSummary(ran.stdout) : null;
      if (summary === null) {
        $.ui.status('tasks: board unavailable');
        if (!failed) $.ui.log(`task-status: task_board.py summary failed: ${ran.stderr.trim().slice(0, 300)}`);
        failed = true;
        return;
      }
      failed = false;
      $.ui.status(summary.text);
      for (const id of newlyWaiting(waiting, summary.needs_you)) {
        $.ui.toast(`${id} needs you (/tasks focus ${id})`);
      }
      waiting = summary.needs_you;
    };

    await poll();
    $.clock.every(POLL_MS, poll);
    return result;
  });
};
