// The sidebar tells views apart by icon: Scheduled once reused Sent's paper
// plane outright, so the two entries looked the same.

import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { ClockIcon, ScheduledSendIcon, SentIcon } from './MailIcons';

describe('MailIcons', () => {
  it('draws Scheduled differently from Sent and from Snoozed', () => {
    const scheduled = renderToStaticMarkup(<ScheduledSendIcon />);
    expect(scheduled).not.toBe(renderToStaticMarkup(<SentIcon />));
    expect(scheduled).not.toBe(renderToStaticMarkup(<ClockIcon />));
  });
});
