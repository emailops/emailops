/**
 * Pre-send checks for a composed message: things the user almost certainly
 * did not mean to send. AI drafts are the main source — they may say "te
 * adjunto…" (a draft cannot attach anything) or leave a `[placeholder]` for a
 * fact the thread did not contain — but the checks apply to any message.
 */
export type SendWarning = { kind: 'missingAttachment' } | { kind: 'unfilledPlaceholder'; text: string };

/** Whole-word phrases (EN/ES/FR/DE) that announce an attached file. */
const ATTACHMENT_WORDS = [
  'attached',
  'enclosed',
  'adjunto',
  'adjunta',
  'adjuntos',
  'adjuntas',
  'adjuntado',
  'adjuntamos',
  'adjunté',
  'ci-joint',
  'ci-jointe',
  'ci-joints',
  'pièce jointe',
  'pièces jointes',
  'anbei',
  'anhang',
  'angehängt',
  'beigefügt',
];

// \p{L} lookarounds: "attached" must not match inside "unattached", nor
// "anhang" inside "zusammenhang".
const ATTACHMENT_RE = new RegExp(`(?<!\\p{L})(${ATTACHMENT_WORDS.join('|')})(?!\\p{L})`, 'iu');

/** A short bracketed note, e.g. `[fecha]` or `[attach: signed contract]`. */
const PLACEHOLDER_RE = /\[([^[\]\n]{1,60})\]/g;

/** The AI's note for a file the user still has to attach. */
const ATTACH_NOTE_RE = /^\s*(attach|adjunt)/i;

export function findSendWarnings(text: string, attachmentCount: number): SendWarning[] {
  const placeholders: string[] = [];
  let attachNote = false;
  for (const match of text.matchAll(PLACEHOLDER_RE)) {
    const inner = match[1].trim();
    if (!inner || /^(https?:|www\.)/i.test(inner)) continue;
    if (ATTACH_NOTE_RE.test(inner)) attachNote = true;
    if (!placeholders.includes(match[0])) placeholders.push(match[0]);
  }

  const warnings: SendWarning[] = [];
  if (attachmentCount === 0 && (attachNote || ATTACHMENT_RE.test(text))) {
    warnings.push({ kind: 'missingAttachment' });
  }
  for (const p of placeholders) {
    warnings.push({ kind: 'unfilledPlaceholder', text: p });
  }
  return warnings;
}
