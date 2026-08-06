import { describe, expect, it } from 'vitest';
import type { LogEntry } from '@/stores/logStore';
import { formatLogsForCopy } from './logExport';

function entry(overrides: Partial<LogEntry> = {}): LogEntry {
  return {
    id: 1,
    // 2026-01-02T03:04:05Z — fixed so the assertion never depends on "now".
    timestamp: Date.UTC(2026, 0, 2, 3, 4, 5),
    level: 'info',
    source: 'sync',
    message: 'Checking for new mail',
    ...overrides,
  };
}

describe('formatLogsForCopy', () => {
  it('puts one entry per line', () => {
    const text = formatLogsForCopy([entry({ id: 1 }), entry({ id: 2, message: 'Done' })]);
    expect(text.split('\n')).toHaveLength(2);
  });

  it('carries the timestamp, level, source and message', () => {
    // Pasting a line into a bug report has to be enough to place it in time and
    // attribute it to a subsystem — the whole reason to copy rather than
    // screenshot.
    const text = formatLogsForCopy([entry()]);
    // Local time, computed the same way the panel does, so this holds in any
    // timezone the suite runs in.
    const d = new Date(entry().timestamp);
    const pad = (n: number) => String(n).padStart(2, '0');
    expect(text).toContain(`${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`);
    expect(text).toContain('info');
    expect(text).toContain('sync');
    expect(text).toContain('Checking for new mail');
  });

  it('keeps entries in the order they were logged', () => {
    const text = formatLogsForCopy([entry({ id: 1, message: 'first' }), entry({ id: 2, message: 'second' })]);
    expect(text.indexOf('first')).toBeLessThan(text.indexOf('second'));
  });

  it('returns an empty string for no entries', () => {
    // The caller disables the button on this, so it must be falsy rather than
    // a header with nothing under it.
    expect(formatLogsForCopy([])).toBe('');
  });

  it('does not let a multi-line message break line-per-entry', () => {
    // Backend errors arrive with embedded newlines (stack-ish provider errors).
    // One entry must stay one line or the paste is unreadable.
    const text = formatLogsForCopy([entry({ message: 'failed:\nconnection reset' })]);
    expect(text.split('\n')).toHaveLength(1);
    expect(text).toContain('connection reset');
  });
});
