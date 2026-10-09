// A Lens cell value that may be long (a summary, a pasted paragraph): clamped
// to three lines with the full text in the tooltip and a toggle to expand it
// in place, so one cell cannot take over the table.

import { useState } from 'react';
import { useTranslation } from 'react-i18next';

const MAX_CHARS = 180;
const MAX_LINES = 3;

/** Whether `text` is likely to overflow the three clamped lines of a cell. */
export function isLongText(text: string): boolean {
  return text.length > MAX_CHARS || text.split('\n').length > MAX_LINES;
}

export function LongText({ text, className }: { text: string; className: string }) {
  const { t } = useTranslation(['lenses']);
  const [expanded, setExpanded] = useState(false);
  const wrap = `${className} whitespace-pre-wrap break-words`;
  if (!isLongText(text)) return <span className={`${wrap} block max-w-[24rem]`}>{text}</span>;
  // `line-clamp-3` sets its own display, so the clamped span must not be `block`.
  // The min width keeps the table from squeezing a long value into a tall, thin column.
  return (
    <span className="block w-max min-w-[16rem] max-w-[24rem]">
      <span className={expanded ? `${wrap} block` : `${wrap} line-clamp-3`} title={text}>
        {text}
      </span>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          setExpanded((v) => !v);
        }}
        className="text-[11px] text-blue-400 hover:underline"
      >
        {expanded ? t('lenses:table.showLess') : t('lenses:table.showMore')}
      </button>
    </span>
  );
}
