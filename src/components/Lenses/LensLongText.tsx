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
  const base = `${className} block max-w-[24rem] whitespace-pre-wrap break-words`;
  if (!isLongText(text)) return <span className={base}>{text}</span>;
  return (
    <span className="block max-w-[24rem]">
      <span className={expanded ? base : `${base} line-clamp-3`} title={text}>
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
