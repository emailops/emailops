#!/usr/bin/env node
// Daily release-metrics collector (`make metrics`, `.github/workflows/metrics.yml`).
//
// GitHub exposes a per-asset `download_count` but no history for it: the API
// only ever tells you today's number. Adoption is the one signal this project
// has — there is no telemetry in the app, by design — so the history has to be
// kept here. Each run appends one row (date, total downloads, stars) to a CSV
// and mails the current numbers plus the deltas that CSV makes computable.
//
// Everything above `main()` is pure: it takes releases, a star count, the
// stored history and today's date, and returns numbers and strings. That is
// what `scripts/repo_metrics.test.mjs` asserts on. `main()` is the thin shell
// that talks to the API, the filesystem and (through `scripts/send_mail.sh`)
// the SMTP server.

import { readFileSync, writeFileSync } from 'node:fs';

const REPO = process.env.METRICS_REPO ?? 'emailops/emailops';
// Overridable so the collector can be pointed at a stub server to rehearse a
// full run — fetch, history, mail file — without touching the real API.
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

function escapeHtml(value) {
  return String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');
}

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

  const html = `<div style="font:14px/1.5 -apple-system,Segoe UI,Roboto,sans-serif;color:#111">
<h2 style="margin:0 0 4px">EmailOps · ${escapeHtml(today)}</h2>
<p style="margin:0 0 16px;font-size:22px">
<strong>${summary.total}</strong> descargas${downloadDelta === null ? '' : ` <span style="color:${downloadDelta > 0 ? '#137333' : '#666'}">${downloadDelta === 0 ? 'sin cambios' : signed(downloadDelta)}</span>`}
&nbsp;·&nbsp; <strong>${stars}</strong> estrellas${starDelta ? ` <span style="color:#137333">${signed(starDelta)}</span>` : ''}
${weekDelta === null ? '' : `<br><span style="font-size:13px;color:#666">7 días: ${signed(weekDelta)}</span>`}
</p>
<table cellpadding="6" style="border-collapse:collapse;font-size:13px">
<tr><th align="left" style="border-bottom:1px solid #ddd">Plataforma</th><th align="right" style="border-bottom:1px solid #ddd">Descargas</th></tr>
${platformRows.map(([k, v]) => `<tr><td>${escapeHtml(PLATFORM_LABELS[k])}</td><td align="right">${v}</td></tr>`).join('\n')}
</table>
<table cellpadding="6" style="border-collapse:collapse;font-size:13px;margin-top:16px">
<tr><th align="left" style="border-bottom:1px solid #ddd">Release</th><th align="right" style="border-bottom:1px solid #ddd">Descargas</th></tr>
${releaseRows.map((r) => `<tr><td>${escapeHtml(r.tag)}</td><td align="right">${r.downloads}</td></tr>`).join('\n')}
</table>
<table cellpadding="6" style="border-collapse:collapse;font-size:13px;margin-top:16px">
<tr><th align="left" style="border-bottom:1px solid #ddd">Fecha</th><th align="right" style="border-bottom:1px solid #ddd">Descargas</th><th align="right" style="border-bottom:1px solid #ddd">Estrellas</th></tr>
${recent.map((r) => `<tr><td>${escapeHtml(r.date)}</td><td align="right">${r.total}</td><td align="right">${r.stars}</td></tr>`).join('\n')}
</table>
<p style="margin-top:16px"><a href="https://github.com/${escapeHtml(REPO)}/releases">Releases en GitHub</a></p>
</div>`;

  return { subject, text, html };
}

/**
 * RFC 5322 message with both parts. Headers and bodies are base64-encoded
 * because the subject and body carry accents and `·` — a raw 8-bit subject
 * arrives as mojibake in most clients.
 */
export function buildMime({ subject, text, html, from, to, date }) {
  const b64 = (s) => Buffer.from(s, 'utf8').toString('base64');
  const wrap = (s) => (s.match(/.{1,76}/g) ?? []).join('\r\n');
  const boundary = 'emailops-metrics-boundary';
  return [
    `From: ${from}`,
    `To: ${to}`,
    `Subject: =?UTF-8?B?${b64(subject)}?=`,
    `Date: ${date}`,
    'MIME-Version: 1.0',
    `Content-Type: multipart/alternative; boundary="${boundary}"`,
    '',
    `--${boundary}`,
    'Content-Type: text/plain; charset=UTF-8',
    'Content-Transfer-Encoding: base64',
    '',
    wrap(b64(text)),
    `--${boundary}`,
    'Content-Type: text/html; charset=UTF-8',
    'Content-Transfer-Encoding: base64',
    '',
    wrap(b64(html)),
    `--${boundary}--`,
    '',
  ].join('\r\n');
}

async function githubJson(path) {
  const headers = { Accept: 'application/vnd.github+json', 'User-Agent': 'emailops-metrics' };
  // Unauthenticated calls are rate-limited to 60/hour per IP, which a shared
  // Actions runner burns through. GITHUB_TOKEN is injected by the workflow.
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  const res = await fetch(`${API}${path}`, { headers });
  if (!res.ok) throw new Error(`GitHub ${path} → HTTP ${res.status} ${res.statusText}`);
  return res.json();
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
  const mailPath = arg(argv, '--out-mail', 'metrics-mail.txt');
  const today = arg(argv, '--today', new Date().toISOString().slice(0, 10));

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
  writeFileSync(
    mailPath,
    buildMime({
      ...report,
      from: process.env.MAIL_FROM ?? 'metrics@localhost',
      to: process.env.MAIL_TO ?? 'metrics@localhost',
      date: new Date().toUTCString(),
    }),
  );
  // Consumed by the workflow to decide whether anything is worth committing.
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
