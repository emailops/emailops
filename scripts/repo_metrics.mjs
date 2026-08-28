#!/usr/bin/env node
// Daily release-metrics collector (`make metrics`, `.github/workflows/metrics.yml`).
//
// GitHub exposes a per-asset `download_count` but no history for it: the API
// only ever tells you today's number. Adoption is the one signal this project
// has — there is no telemetry in the app, by design — so the history has to be
// kept here. Each run appends one row (date, total downloads, stars) to a CSV
// and posts the current numbers, plus the deltas that CSV makes computable, as
// a comment on a long-lived issue — GitHub's own notification mail delivers it,
// so there is no SMTP secret to configure anywhere.
//
// Everything above `main()` is pure: it takes releases, a star count, the
// stored history and today's date, and returns numbers and strings. That is
// what `scripts/repo_metrics.test.mjs` asserts on. `main()` is the thin shell
// that talks to the API and the filesystem.

import { readFileSync, writeFileSync } from 'node:fs';

const REPO = process.env.METRICS_REPO ?? 'emailops/emailops';
// Overridable so the collector can be pointed at a stub server to rehearse a
// full run — fetch, history, comment — without touching the real API.
const API = process.env.METRICS_API ?? 'https://api.github.com';

/** Assets whose name matches one of these belongs to that platform bucket. */
const PLATFORM_RULES = [
  // `EmailOps-CLI-macos.dmg` is a macOS disk image too, so the CLI rule has to
  // be tested first or every CLI download reads as a desktop-app install.
  [/(^|[-_])cli[-_]/i, 'cli'],
  [/\.(dmg)$/i, 'macos'],
  [/\.(msi|exe)$/i, 'windows'],
  [/\.(appimage|deb|rpm)$/i, 'linux'],
];

export function classifyAsset(name) {
  for (const [pattern, platform] of PLATFORM_RULES) {
    if (pattern.test(name)) return platform;
  }
  return 'other';
}

/**
 * Fold a release list into today's totals: the grand total, a per-platform
 * split, and a per-release split in the order GitHub returned (newest first).
 */
export function summarize(releases) {
  const byPlatform = { macos: 0, windows: 0, linux: 0, cli: 0, other: 0 };
  const byRelease = [];
  let total = 0;

  for (const rel of releases) {
    let downloads = 0;
    for (const asset of rel.assets ?? []) {
      const count = asset.download_count ?? 0;
      downloads += count;
      byPlatform[classifyAsset(asset.name)] += count;
    }
    byRelease.push({ tag: rel.tag_name, downloads });
    total += downloads;
  }

  return { total, byPlatform, byRelease };
}

const HEADER = 'date,total,stars';

export function parseHistory(csv) {
  return csv
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line !== '' && line !== HEADER)
    .map((line) => {
      const [date, total, stars] = line.split(',');
      return { date, total: Number(total), stars: Number(stars) };
    });
}

export function serializeHistory(rows) {
  return `${[HEADER, ...rows.map((r) => `${r.date},${r.total},${r.stars}`)].join('\n')}\n`;
}

/**
 * Add today's row, or correct it in place when the day already has one — a
 * manual re-run after the scheduled job must not leave two rows for one date.
 */
export function upsertRow(rows, row) {
  const kept = rows.filter((r) => r.date !== row.date);
  return [...kept, row].sort((a, b) => a.date.localeCompare(b.date));
}

/** Row from `daysBack` days ago (or the closest one before it), if any. */
function baselineRow(history, today, daysBack) {
  const cutoff = new Date(`${today}T00:00:00Z`);
  cutoff.setUTCDate(cutoff.getUTCDate() - daysBack);
  const iso = cutoff.toISOString().slice(0, 10);
  const candidates = history.filter((r) => r.date <= iso);
  return candidates.length > 0 ? candidates[candidates.length - 1] : null;
}

const signed = (n) => (n >= 0 ? `+${n}` : `${n}`);

/** A literal `|` or newline inside a cell would break the markdown table. */
function escapeCell(value) {
  return String(value).replaceAll('|', '\\|').replaceAll('\n', ' ');
}

/** Title of the long-lived issue the daily report is posted to. */
export const ISSUE_TITLE = '📊 Métricas de descargas y estrellas';

const PLATFORM_LABELS = { macos: 'macOS', windows: 'Windows', linux: 'Linux', cli: 'CLI', other: 'Otros' };

/**
 * Render the daily mail. `history` must already include today's row, so the
 * previous row is the day-over-day baseline. On the very first run there is no
 * previous row and every delta is omitted — a cold start is not a spike.
 */
export function buildReport({ summary, stars, history, today }) {
  const previous = history.filter((r) => r.date < today).slice(-1)[0] ?? null;
  const week = previous ? baselineRow(history, today, 7) : null;

  const downloadDelta = previous ? summary.total - previous.total : null;
  const starDelta = previous ? stars - previous.stars : null;
  const weekDelta = week ? summary.total - week.total : null;

  let downloadsPart = `${summary.total} descargas`;
  if (downloadDelta !== null) downloadsPart += downloadDelta === 0 ? ' (sin cambios)' : ` (${signed(downloadDelta)})`;
  let starsPart = `${stars} estrellas`;
  if (starDelta !== null && starDelta !== 0) starsPart += ` (${signed(starDelta)})`;
  const subject = `EmailOps · ${downloadsPart} · ${starsPart}`;

  const platformRows = Object.entries(summary.byPlatform)
    .filter(([, count]) => count > 0)
    .sort((a, b) => b[1] - a[1]);
  const releaseRows = summary.byRelease.filter((r) => r.downloads > 0);
  const recent = history.slice(-14);

  const lines = [
    `EmailOps — ${today}`,
    '',
    `Descargas: ${summary.total}${downloadDelta === null ? '' : ` (${downloadDelta === 0 ? 'sin cambios' : signed(downloadDelta)} desde ayer)`}`,
  ];
  if (weekDelta !== null) lines.push(`7 días: ${signed(weekDelta)}`);
  lines.push(`Estrellas: ${stars}${starDelta === null || starDelta === 0 ? '' : ` (${signed(starDelta)})`}`);
  lines.push('', 'Por plataforma:');
  for (const [key, count] of platformRows) lines.push(`  ${PLATFORM_LABELS[key]}: ${count}`);
  lines.push('', 'Por release:');
  for (const rel of releaseRows) lines.push(`  ${rel.tag}: ${rel.downloads}`);
  lines.push('', 'Últimos días:');
  for (const row of recent) lines.push(`  ${row.date}  ${row.total} descargas  ${row.stars} estrellas`);
  lines.push('', `https://github.com/${REPO}/releases`);
  const text = lines.join('\n');

  const table = (header, rows) => [`| ${header[0]} | ${header[1]} |`, '|---|---:|', ...rows].join('\n');
  const markdown = [
    `**${summary.total} descargas**${downloadDelta === null ? '' : ` (${downloadDelta === 0 ? 'sin cambios' : signed(downloadDelta)})`} · **${stars} estrellas**${starDelta ? ` (${signed(starDelta)})` : ''}`,
    '',
    ...(weekDelta === null ? [] : [`Últimos 7 días: **${signed(weekDelta)}**`, '']),
    table(
      ['Plataforma', 'Descargas'],
      platformRows.map(([k, v]) => `| ${PLATFORM_LABELS[k]} | ${v} |`),
    ),
    '',
    '<details><summary>Por release</summary>',
    '',
    table(
      ['Release', 'Descargas'],
      releaseRows.map((r) => `| ${escapeCell(r.tag)} | ${r.downloads} |`),
    ),
    '',
    '</details>',
    '',
    '<details><summary>Últimos días</summary>',
    '',
    [
      '| Fecha | Descargas | Estrellas |',
      '|---|---:|---:|',
      ...recent.map((r) => `| ${r.date} | ${r.total} | ${r.stars} |`),
    ].join('\n'),
    '',
    '</details>',
    '',
    // Invisible in the rendered issue and in the notification mail; lets a
    // later run (or a human) read the day's numbers straight off the comment.
    `<!-- emailops-metrics total=${summary.total} stars=${stars} -->`,
  ].join('\n');

  return { subject, text, markdown };
}

async function githubJson(path, { method = 'GET', body } = {}) {
  const headers = { Accept: 'application/vnd.github+json', 'User-Agent': 'emailops-metrics' };
  // Reads are rate-limited to 60/hour per IP unauthenticated, which a shared
  // Actions runner burns through, and writes need a token outright.
  // GITHUB_TOKEN is injected by the workflow — no configured secret involved.
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  if (body) headers['Content-Type'] = 'application/json';
  const res = await fetch(`${API}${path}`, { method, headers, body: body ? JSON.stringify(body) : undefined });
  if (!res.ok) throw new Error(`GitHub ${method} ${path} → HTTP ${res.status} ${res.statusText}`);
  return res.json();
}

/**
 * The single long-lived issue the daily report is posted to. The `issues`
 * endpoint also returns pull requests, which must never be matched.
 */
export function findMetricsIssue(issues, title) {
  return issues.find((issue) => issue.title === title && !issue.pull_request) ?? null;
}

/**
 * Post the day's report as a comment on that issue, opening it on first run.
 * Delivery is GitHub's own notification mail: the repo owner watches the repo,
 * so every comment on the thread reaches their inbox. That is the whole reason
 * this is an issue comment and not an SMTP message — no secret to configure,
 * nothing to rotate, and the thread doubles as a readable archive.
 */
async function postReport(markdown, title) {
  const open = await githubJson(`/repos/${REPO}/issues?state=open&per_page=100`);
  let issue = findMetricsIssue(open, title);
  if (!issue) {
    issue = await githubJson(`/repos/${REPO}/issues`, {
      method: 'POST',
      body: {
        title,
        body: [
          'Informe diario de descargas y estrellas, publicado por',
          '[`.github/workflows/metrics.yml`](../blob/main/.github/workflows/metrics.yml).',
          '',
          'Cada día aparece aquí un comentario nuevo — y con él, la notificación de',
          'GitHub en el correo. Cierra este issue para dejar de recibirlo: el',
          'workflow abrirá otro en la siguiente ejecución, así que deshabilítalo en',
          'la pestaña Actions si lo que quieres es pararlo del todo.',
        ].join('\n'),
      },
    });
    console.log(`[metrics] opened issue #${issue.number}`);
  }
  const comment = await githubJson(`/repos/${REPO}/issues/${issue.number}/comments`, {
    method: 'POST',
    body: { body: markdown },
  });
  console.log(`[metrics] posted ${comment.html_url}`);
}

async function fetchAllReleases() {
  const all = [];
  for (let page = 1; ; page += 1) {
    const batch = await githubJson(`/repos/${REPO}/releases?per_page=100&page=${page}`);
    all.push(...batch);
    if (batch.length < 100) return all;
  }
}

function arg(argv, name, fallback) {
  const i = argv.indexOf(name);
  return i === -1 ? fallback : argv[i + 1];
}

async function main(argv) {
  const dryRun = argv.includes('--dry-run');
  const historyPath = arg(argv, '--history', 'downloads.csv');
  const today = arg(argv, '--today', new Date().toISOString().slice(0, 10));
  const issueTitle = arg(argv, '--issue-title', ISSUE_TITLE);

  const [releases, repo] = await Promise.all([fetchAllReleases(), githubJson(`/repos/${REPO}`)]);
  const summary = summarize(releases);
  const stars = repo.stargazers_count;

  let stored = [];
  try {
    stored = parseHistory(readFileSync(historyPath, 'utf8'));
  } catch (err) {
    // A missing file is the expected first run; anything else (unreadable,
    // corrupt) must surface rather than silently resetting the history.
    if (err.code !== 'ENOENT') throw err;
    console.error(`[metrics] no history at ${historyPath} — starting a new one`);
  }

  const history = upsertRow(stored, { date: today, total: summary.total, stars });
  const report = buildReport({ summary, stars, history, today });

  if (dryRun) {
    console.log(report.subject);
    console.log('');
    console.log(report.text);
    return;
  }

  writeFileSync(historyPath, serializeHistory(history));
  await postReport(report.markdown, issueTitle);

  const previous = history.filter((r) => r.date < today).slice(-1)[0] ?? null;
  const changed = !previous || previous.total !== summary.total || previous.stars !== stars;
  console.log(`total=${summary.total} stars=${stars} changed=${changed}`);
}

// Only run when invoked as a script — importing this file from the tests must
// not hit the network.
if (process.argv[1]?.endsWith('repo_metrics.mjs')) {
  main(process.argv.slice(2)).catch((err) => {
    console.error(`[metrics] ${err.message}`);
    process.exit(1);
  });
}
