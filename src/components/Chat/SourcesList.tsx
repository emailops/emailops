import { useState } from 'react';
import { useFormatters } from '@/hooks/useFormatters';
import type { ChatMessageSource } from '@/types';
import { CitationPill } from './CitationPill';

function SourceRow({
  source,
  accountId,
  onOpenEmail,
}: {
  source: ChatMessageSource;
  accountId: string;
  onOpenEmail?: () => void;
}) {
  const fmt = useFormatters();
  const [showChunk, setShowChunk] = useState(false);
  const hasExcerpt = !!source.bodyExcerpt && source.bodyExcerpt.length > 0;
  return (
    <li className="text-xs text-gray-600 py-1 px-1 rounded hover:bg-gray-50">
      <div className="flex items-start gap-2">
        <CitationPill source={source} accountId={accountId} onOpenEmail={onOpenEmail} />
        <div className="flex-1 min-w-0">
          <div className="font-medium truncate">{source.subject || '(no subject)'}</div>
          <div className="text-gray-400 truncate">
            {source.sender || source.senderEmail}
            {source.timestamp
              ? ` · ${fmt.date(source.timestamp, { month: 'short', day: 'numeric', year: 'numeric' })}`
              : ''}
          </div>
          {hasExcerpt && (
            <button
              type="button"
              onClick={() => setShowChunk((v) => !v)}
              className="mt-0.5 flex items-center gap-1 text-[11px] text-gray-500 hover:text-gray-700"
            >
              <svg
                className={`w-3 h-3 transition-transform ${showChunk ? 'rotate-90' : ''}`}
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
              </svg>
              {showChunk ? 'Hide chunk' : 'Show chunk'}
            </button>
          )}
        </div>
      </div>
      {hasExcerpt && showChunk && (
        <pre className="mt-1 ml-6 p-1.5 rounded bg-gray-50 border border-gray-200 text-[11px] text-gray-700 whitespace-pre-wrap break-words">
          {source.bodyExcerpt}
        </pre>
      )}
    </li>
  );
}

export function SourcesList({
  sources,
  accountId,
  onOpenEmail,
}: {
  sources: ChatMessageSource[];
  accountId: string;
  onOpenEmail?: () => void;
}) {
  const [isOpen, setIsOpen] = useState(false);

  if (sources.length === 0) return null;

  return (
    <div className="mt-2 pt-2 border-t border-gray-200">
      <button
        type="button"
        onClick={() => setIsOpen((v) => !v)}
        className="flex items-center gap-1 text-xs text-gray-500 hover:text-gray-700 transition-colors"
      >
        <svg
          className={`w-3 h-3 transition-transform ${isOpen ? 'rotate-90' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
        </svg>
        {sources.length} source{sources.length !== 1 ? 's' : ''} used
      </button>

      {isOpen && (
        <ul className="mt-1.5 space-y-1">
          {sources.map((src) => (
            <SourceRow key={src.citationNumber} source={src} accountId={accountId} onOpenEmail={onOpenEmail} />
          ))}
        </ul>
      )}
    </div>
  );
}
