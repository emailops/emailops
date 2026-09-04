// Unit tests for the pure core of `scripts/repo_metrics.mjs` — the daily
// release-metrics collector behind `.github/workflows/metrics.yml`.
//
// Everything asserted here is a pure function over data the caller supplies:
// no network, no filesystem, no clock. The thin I/O shell (fetching the API,
// reading/writing the CSV, handing the mail to curl) lives in `main()` and is
// exercised by `make metrics` instead.

import { describe, expect, it } from 'vitest';
import {
  buildReport,
  classifyAsset,
  findMetricsIssue,
  mergeReferrerHistory,
  mergeTrafficHistory,
  parseHistory,
  parseTrafficHistory,
  serializeHistory,
  serializeTrafficHistory,
  summarize,
  trafficRows,
  upsertRow,
} from './repo_metrics.mjs';

/** Minimal release shape — only the fields the collector reads. */
function release(tag, assets) {
  return { tag_name: tag, assets: assets.map(([name, download_count]) => ({ name, download_count })) };
}

describe('classifyAsset', () => {
  it.each([
    ['EmailOps-macos.dmg', 'macos'],
    ['EmailOps_0.5.0_universal.dmg', 'macos'],
    ['EmailOps-macos-intel.dmg', 'macos'],
    ['EmailOps-windows.msi', 'windows'],
    ['EmailOps-windows-setup.exe', 'windows'],
    ['EmailOps-windows-cuda.msi', 'windows'],
    ['EmailOps-linux.AppImage', 'linux'],
    ['EmailOps-linux.deb', 'linux'],
  ])('classifies %s as %s', (name, expected) => {
    expect(classifyAsset(name)).toBe(expected);
  });

  // The CLI disk image is also a macOS .dmg, so the CLI rule has to win or
  // every CLI download would be miscounted as a desktop-app install.
  it('counts the CLI disk image as cli, not macos', () => {
    expect(classifyAsset('EmailOps-CLI-macos.dmg')).toBe('cli');
  });

  it('falls back to other for an unrecognised asset', () => {
    expect(classifyAsset('checksums.txt')).toBe('other');
  });
});

describe('summarize', () => {
  const releases = [
    release('v0.6.6', [
      ['EmailOps-macos.dmg', 15],
      ['EmailOps-CLI-macos.dmg', 2],
      ['EmailOps-windows.msi', 1],
      ['EmailOps-linux.deb', 2],
    ]),
    release('v0.6.5', [
      ['EmailOps-macos.dmg', 14],
      ['EmailOps-linux.AppImage', 6],
    ]),
  ];

  it('totals every asset of every release', () => {
    expect(summarize(releases).total).toBe(40);
  });

  it('breaks the total down by platform', () => {
    expect(summarize(releases).byPlatform).toEqual({
      macos: 29,
      windows: 1,
      linux: 8,
      cli: 2,
      other: 0,
    });
  });

  it('breaks the total down by release, newest first', () => {
    expect(summarize(releases).byRelease).toEqual([
      { tag: 'v0.6.6', downloads: 20 },
      { tag: 'v0.6.5', downloads: 20 },
    ]);
  });

  it('treats a release with no assets as zero rather than failing', () => {
    expect(summarize([release('v0.1.0', [])]).total).toBe(0);
  });
});

describe('history round-trip', () => {
  const rows = [
    { date: '2026-08-25', total: 124, stars: 11 },
    { date: '2026-08-26', total: 127, stars: 12 },
  ];

  it('parses what it serializes', () => {
    expect(parseHistory(serializeHistory(rows))).toEqual(rows);
  });

  it('reads an empty file as no history', () => {
    expect(parseHistory('')).toEqual([]);
  });

  it('ignores a trailing newline and the header row', () => {
    expect(parseHistory('date,total,stars\n2026-08-25,124,11\n')).toEqual([
      { date: '2026-08-25', total: 124, stars: 11 },
    ]);
  });
});

describe('upsertRow', () => {
  const rows = [{ date: '2026-08-25', total: 124, stars: 11 }];

  it('appends a new day', () => {
    expect(upsertRow(rows, { date: '2026-08-26', total: 127, stars: 12 })).toEqual([
      { date: '2026-08-25', total: 124, stars: 11 },
      { date: '2026-08-26', total: 127, stars: 12 },
    ]);
  });

  // A re-run on the same day (manual dispatch after the scheduled one) must
  // correct that day's row, not add a second one for the same date.
  it('replaces the row for a day it already has', () => {
    expect(upsertRow(rows, { date: '2026-08-25', total: 130, stars: 11 })).toEqual([
      { date: '2026-08-25', total: 130, stars: 11 },
    ]);
  });

  it('keeps rows sorted by date when a backfill lands out of order', () => {
    expect(upsertRow(rows, { date: '2026-08-20', total: 100, stars: 9 }).map((r) => r.date)).toEqual([
      '2026-08-20',
      '2026-08-25',
    ]);
  });
});

describe('buildReport', () => {
  const summary = {
    total: 127,
    byPlatform: { macos: 80, windows: 24, linux: 20, cli: 3, other: 0 },
    byRelease: [
      { tag: 'v0.6.6', downloads: 38 },
      { tag: 'v0.6.5', downloads: 37 },
    ],
  };
  const history = [
    { date: '2026-08-19', total: 118, stars: 10 },
    { date: '2026-08-25', total: 124, stars: 11 },
    { date: '2026-08-26', total: 127, stars: 12 },
  ];

  it('puts the total and the day-over-day delta in the subject', () => {
    const { subject } = buildReport({ summary, stars: 12, history, today: '2026-08-26' });
    expect(subject).toBe('EmailOps · 127 descargas (+3) · 12 estrellas (+1)');
  });

  it('says "sin cambios" in the subject when nothing moved', () => {
    const flat = [
      { date: '2026-08-25', total: 127, stars: 12 },
      { date: '2026-08-26', total: 127, stars: 12 },
    ];
    const { subject } = buildReport({ summary, stars: 12, history: flat, today: '2026-08-26' });
    expect(subject).toBe('EmailOps · 127 descargas (sin cambios) · 12 estrellas');
  });

  it('reports the 7-day delta alongside the daily one', () => {
    const { text } = buildReport({ summary, stars: 12, history, today: '2026-08-26' });
    expect(text).toContain('7 días: +9');
  });

  // First run: there is no previous row to compare against, and inventing a
  // delta of +127 would misread a cold start as a spike.
  it('omits deltas entirely on the very first run', () => {
    const first = [{ date: '2026-08-26', total: 127, stars: 12 }];
    const { subject, text } = buildReport({ summary, stars: 12, history: first, today: '2026-08-26' });
    expect(subject).toBe('EmailOps · 127 descargas · 12 estrellas');
    expect(text).not.toContain('7 días');
  });

  it('breaks the total down by platform and by release in the body', () => {
    const { text } = buildReport({ summary, stars: 12, history, today: '2026-08-26' });
    expect(text).toContain('macOS');
    expect(text).toContain('80');
    expect(text).toContain('v0.6.6');
  });

  it('leads the markdown with the headline numbers, so they show in the mail preview', () => {
    const { markdown } = buildReport({ summary, stars: 12, history, today: '2026-08-26' });
    expect(markdown.split('\n')[0]).toBe('**127 descargas** (+3) · **12 estrellas** (+1)');
  });

  it('renders the platform split as a markdown table', () => {
    const { markdown } = buildReport({ summary, stars: 12, history, today: '2026-08-26' });
    expect(markdown).toContain('| macOS | 80 |');
  });

  // The marker lets a later run recognise its own comments, and is invisible
  // in both the issue and the notification mail.
  it('ends with a machine-readable marker carrying the totals', () => {
    const { markdown } = buildReport({ summary, stars: 12, history, today: '2026-08-26' });
    expect(markdown.trimEnd()).toMatch(/<!-- emailops-metrics total=127 stars=12 -->$/);
  });
});

describe('findMetricsIssue', () => {
  const title = '📊 Métricas de descargas y estrellas';

  it('finds the open issue with the metrics title', () => {
    const issues = [
      { number: 3, title: 'Otro asunto' },
      { number: 7, title },
    ];
    expect(findMetricsIssue(issues, title)?.number).toBe(7);
  });

  // The issues endpoint returns pull requests too; commenting on a PR instead
  // of the metrics thread would be wrong and confusing.
  it('never matches a pull request', () => {
    const issues = [{ number: 9, title, pull_request: { url: 'https://…' } }];
    expect(findMetricsIssue(issues, title)).toBeNull();
  });

  it('returns null when the issue does not exist yet', () => {
    expect(findMetricsIssue([], title)).toBeNull();
  });
});

describe('traffic', () => {
  // Shape of /traffic/views and /traffic/clones: a total, a unique count, and
  // a per-day breakdown. Only the per-day rows are archived — the totals are
  // a rolling 14-day sum and mean nothing once stitched into a history.
  const views = {
    count: 120,
    uniques: 40,
    views: [
      { timestamp: '2026-09-03T00:00:00Z', count: 50, uniques: 18 },
      { timestamp: '2026-09-04T00:00:00Z', count: 70, uniques: 22 },
    ],
  };
  const clones = {
    count: 9,
    uniques: 5,
    clones: [
      { timestamp: '2026-09-03T00:00:00Z', count: 4, uniques: 2 },
      { timestamp: '2026-09-04T00:00:00Z', count: 5, uniques: 3 },
    ],
  };

  it('turns the API payloads into one row per day', () => {
    expect(trafficRows(views, clones)).toEqual([
      { date: '2026-09-03', views: 50, unique_views: 18, clones: 4, unique_clones: 2 },
      { date: '2026-09-04', views: 70, unique_views: 22, clones: 5, unique_clones: 3 },
    ]);
  });

  // GitHub reports views and clones on independent day lists: a day with
  // traffic but no clone appears in one and not the other.
  it('fills a day that only one of the two endpoints reports', () => {
    const lonely = { count: 1, uniques: 1, clones: [{ timestamp: '2026-09-05T00:00:00Z', count: 1, uniques: 1 }] };
    expect(trafficRows(views, lonely)).toEqual([
      { date: '2026-09-03', views: 50, unique_views: 18, clones: 0, unique_clones: 0 },
      { date: '2026-09-04', views: 70, unique_views: 22, clones: 0, unique_clones: 0 },
      { date: '2026-09-05', views: 0, unique_views: 0, clones: 1, unique_clones: 1 },
    ]);
  });

  it('round-trips a traffic history through CSV', () => {
    const rows = trafficRows(views, clones);
    expect(parseTrafficHistory(serializeTrafficHistory(rows))).toEqual(rows);
  });

  // The 14-day window is the whole reason this exists: a re-fetched day must
  // correct the stored row, and days that scrolled out of the window must
  // survive in the history.
  it('merges a fresh window over the stored history, keeping older days', () => {
    const stored = [
      { date: '2026-08-01', views: 5, unique_views: 2, clones: 0, unique_clones: 0 },
      { date: '2026-09-03', views: 1, unique_views: 1, clones: 0, unique_clones: 0 },
    ];
    expect(mergeTrafficHistory(stored, trafficRows(views, clones))).toEqual([
      { date: '2026-08-01', views: 5, unique_views: 2, clones: 0, unique_clones: 0 },
      { date: '2026-09-03', views: 50, unique_views: 18, clones: 4, unique_clones: 2 },
      { date: '2026-09-04', views: 70, unique_views: 22, clones: 5, unique_clones: 3 },
    ]);
  });

  it('appends dated referrer snapshots without losing earlier ones', () => {
    const stored = [{ date: '2026-09-03', referrer: 'google.com', count: 4, uniques: 3 }];
    const fresh = [
      { referrer: 'news.ycombinator.com', count: 30, uniques: 25 },
      { referrer: 'google.com', count: 6, uniques: 4 },
    ];
    expect(mergeReferrerHistory(stored, fresh, '2026-09-04')).toEqual([
      { date: '2026-09-03', referrer: 'google.com', count: 4, uniques: 3 },
      { date: '2026-09-04', referrer: 'news.ycombinator.com', count: 30, uniques: 25 },
      { date: '2026-09-04', referrer: 'google.com', count: 6, uniques: 4 },
    ]);
  });

  it('replaces a same-day referrer snapshot on a re-run', () => {
    const stored = [{ date: '2026-09-04', referrer: 'google.com', count: 1, uniques: 1 }];
    const fresh = [{ referrer: 'google.com', count: 6, uniques: 4 }];
    expect(mergeReferrerHistory(stored, fresh, '2026-09-04')).toEqual([
      { date: '2026-09-04', referrer: 'google.com', count: 6, uniques: 4 },
    ]);
  });
});

describe('buildReport with traffic', () => {
  const summary = {
    total: 200,
    byPlatform: { macos: 104, windows: 58, linux: 35, cli: 3, other: 0 },
    byRelease: [{ tag: 'v0.6.6', downloads: 60 }],
  };
  const history = [
    { date: '2026-09-03', total: 188, stars: 12 },
    { date: '2026-09-04', total: 200, stars: 12 },
  ];
  const traffic = {
    today: { date: '2026-09-04', views: 70, unique_views: 22, clones: 5, unique_clones: 3 },
    referrers: [
      { referrer: 'news.ycombinator.com', count: 30, uniques: 25 },
      { referrer: 'google.com', count: 6, uniques: 4 },
    ],
  };

  it('reports visits and the top referrers when traffic is available', () => {
    const { markdown } = buildReport({ summary, stars: 12, history, today: '2026-09-04', traffic });
    expect(markdown).toContain('70');
    expect(markdown).toContain('news.ycombinator.com');
  });

  // The traffic API needs push access and may answer 403. That must degrade to
  // a report without a traffic section, never to a failed run.
  it('omits the traffic section entirely when traffic is unavailable', () => {
    const { markdown } = buildReport({ summary, stars: 12, history, today: '2026-09-04', traffic: null });
    expect(markdown).not.toContain('Visitas');
    expect(markdown).toContain('200 descargas');
  });
});
