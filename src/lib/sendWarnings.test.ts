import { describe, expect, it } from 'vitest';
import { findSendWarnings } from './sendWarnings';

describe('findSendWarnings — missing attachment', () => {
  it.each([
    'Hola Ana, te adjunto el contrato firmado.',
    'Hi Bob, please find attached the invoice.',
    "I've attached the slides.",
    'Bonjour, vous trouverez ci-joint le devis.',
    'Voici la pièce jointe demandée.',
    'Hallo, anbei die Rechnung.',
    'Die Unterlagen sind im Anhang.',
  ])('warns when "%s" is sent with no attachment', (text) => {
    expect(findSendWarnings(text, 0)).toContainEqual({ kind: 'missingAttachment' });
  });

  it('does not warn when a file is attached', () => {
    expect(findSendWarnings('Te adjunto el contrato.', 1)).toEqual([]);
  });

  it('does not warn on text that never mentions an attachment', () => {
    expect(findSendWarnings('Perfecto, nos vemos el jueves a las 10:30.', 0)).toEqual([]);
  });

  it('ignores words that merely contain the keyword', () => {
    // "attachment" to a place / "adjuntía" are not attachments; keep the
    // match on whole words so ordinary prose does not nag.
    expect(findSendWarnings('Thanks for your attachment to the project.', 0)).toEqual([]);
  });
});

describe('findSendWarnings — unfilled placeholders', () => {
  it('warns about a placeholder the AI left for the user', () => {
    expect(findSendWarnings('Nos vemos el [fecha] en la oficina.', 0)).toContainEqual({
      kind: 'unfilledPlaceholder',
      text: '[fecha]',
    });
  });

  it('treats an [attach: …] note as both a placeholder and a missing attachment', () => {
    const warnings = findSendWarnings('Hola, [attach: modelo 037]. Un saludo', 0);
    expect(warnings).toContainEqual({ kind: 'unfilledPlaceholder', text: '[attach: modelo 037]' });
    expect(warnings).toContainEqual({ kind: 'missingAttachment' });
  });

  it('ignores links and long bracketed text', () => {
    expect(findSendWarnings('See [https://example.com/docs] for details.', 0)).toEqual([]);
    expect(findSendWarnings(`Note [${'x'.repeat(80)}]`, 0)).toEqual([]);
  });

  it('reports each placeholder once', () => {
    const warnings = findSendWarnings('[nombre], confirmo [fecha]. Gracias, [nombre]', 0);
    expect(warnings.filter((w) => w.kind === 'unfilledPlaceholder')).toHaveLength(2);
  });
});
