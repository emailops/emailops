import { describe, expect, it } from 'vitest';
import { DEFAULT_UNDO_SEND_DELAY, parseUndoSendDelay, restoredBodyHtml, scheduleSendPresets } from './outbox';

// Local-time constructor: the presets are wall-clock times in the user's zone.
const at = (y: number, mo: number, d: number, h = 0, mi = 0) => new Date(y, mo - 1, d, h, mi);

describe('parseUndoSendDelay', () => {
  it('defaults to 10 s when the preference is unset or not an offered window', () => {
    expect(DEFAULT_UNDO_SEND_DELAY).toBe(10);
    for (const raw of [null, '', 'abc', '7', '-5', '60']) {
      expect(parseUndoSendDelay(raw)).toBe(10);
    }
  });

  it('reads off and every offered window', () => {
    expect(parseUndoSendDelay('0')).toBe(0);
    for (const secs of [5, 10, 20, 30]) expect(parseUndoSendDelay(String(secs))).toBe(secs);
  });
});

describe('scheduleSendPresets', () => {
  it('offers tomorrow morning, tomorrow afternoon and Monday morning on a weekday', () => {
    // Wednesday 1 Oct 2025, 15:20
    const presets = scheduleSendPresets(at(2025, 10, 1, 15, 20));
    expect(presets.map((p) => [p.id, p.at])).toEqual([
      ['tomorrowMorning', at(2025, 10, 2, 8)],
      ['tomorrowAfternoon', at(2025, 10, 2, 13)],
      ['mondayMorning', at(2025, 10, 6, 8)],
    ]);
  });

  it('on a Sunday leaves Monday morning out: it is tomorrow morning', () => {
    const presets = scheduleSendPresets(at(2025, 10, 5, 9));
    expect(presets.map((p) => p.id)).toEqual(['tomorrowMorning', 'tomorrowAfternoon']);
  });

  it('on a Monday, Monday morning is next week', () => {
    const presets = scheduleSendPresets(at(2025, 10, 6, 7));
    expect(presets.find((p) => p.id === 'mondayMorning')?.at).toEqual(at(2025, 10, 13, 8));
  });

  it('keeps the wall-clock hour across a month end', () => {
    const presets = scheduleSendPresets(at(2025, 1, 31, 22));
    expect(presets[0].at).toEqual(at(2025, 2, 1, 8));
  });
});

describe('restoredBodyHtml', () => {
  const img = { filename: 'a.png', mimeType: 'image/png', data: 'AAAA', contentId: 'img-1', isInline: true };

  it('puts the inline images back as data URIs for the editor', () => {
    expect(restoredBodyHtml({ body: 'x', bodyHtml: '<p>hi</p><img src="cid:img-1">', inlineImages: [img] })).toBe(
      '<p>hi</p><img src="data:image/png;base64,AAAA">',
    );
  });

  it('turns a plain-text message into paragraphs', () => {
    expect(restoredBodyHtml({ body: 'one\ntwo', bodyHtml: null, inlineImages: [] })).toContain('one');
  });
});
