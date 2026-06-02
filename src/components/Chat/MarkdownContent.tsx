import { Children, isValidElement, type ReactNode, useEffect, useRef } from 'react';
import ReactMarkdown, { defaultUrlTransform } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { useLogStore } from '@/stores/logStore';
import type { ChatMessageSource } from '@/types';
import { CitationPill } from './CitationPill';
import { DraftRefPill } from './DraftRefPill';
import { EmailRefPill } from './EmailRefPill';

/** react-markdown 10 ships a `urlTransform` whose safe-protocol allowlist
 *  is `https?|ircs?|mailto|xmpp` — anything else (including our four
 *  custom schemes) gets rewritten to "". That silently empties every
 *  citation/attachment/email/draft href before the custom `a` renderer
 *  ever sees it, so all four chip types render as plain text. We override
 *  it to pass our schemes through verbatim and delegate everything else to
 *  the upstream default (which still strips `javascript:` and friends). */
const CHAT_URI_SCHEMES = ['citation://', 'attachment://', 'email://', 'draft://'] as const;
function chatUrlTransform(url: string): string {
  if (CHAT_URI_SCHEMES.some((s) => url.startsWith(s))) return url;
  return defaultUrlTransform(url);
}

/** Paragraph renderer extracted as a named function so the `<li>` override
 *  below can compare element types by identity (`c.type === Paragraph`).
 *  The function reference is what react-markdown puts on rendered `<p>`
 *  children — comparing against the string `'p'` would always fail. */
function Paragraph({ children }: { children?: ReactNode }) {
  return <p className="mb-1 last:mb-0">{children}</p>;
}

/** Flatten any `<p>`-rendered children of an `<li>` into the `<li>`
 *  directly. remark renders multi-line or blank-separated list items
 *  "loose" — `<li><p>…</p></li>` — which combined with our block-level
 *  `<p>` renderer pushes the chip below the marker. Whitespace text nodes
 *  around the `<p>` (from remark's serialization) pass through unchanged
 *  so list items that genuinely mix prose and paragraphs still render
 *  with their non-paragraph children intact. */
function unwrapLooseListParagraph(children: ReactNode): ReactNode {
  return Children.map(children, (c) => {
    if (isValidElement<{ children?: ReactNode }>(c) && c.type === Paragraph) {
      return c.props.children;
    }
    return c;
  });
}

/** Pre-process raw content before handing to react-markdown.
 *  Converts bare `[n]` citation markers to `[n](citation://n)` so
 *  react-markdown treats them as links that we can intercept in the
 *  custom `a` renderer below.
 */
function preprocessContent(content: string): string {
  // Match [n] that is NOT already followed by ( (i.e., not already a link)
  return content.replace(/\[(\d+)\](?!\()/g, '[$1](citation://$1)');
}

function AttachmentChip({ label, onOpen }: { label: string; onOpen: () => void }) {
  return (
    <button
      type="button"
      onClick={onOpen}
      className="inline-flex items-center gap-1 px-1.5 py-0.5 mx-0.5 rounded bg-primary-50 border border-primary-200 text-primary-700 text-xs font-medium hover:bg-primary-100 transition-colors align-baseline"
      title={`Open ${label}`}
    >
      <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"
        />
      </svg>
      <span className="truncate max-w-[180px]">{label}</span>
    </button>
  );
}

export function MarkdownContent({
  content,
  sources,
  accountId,
  onOpenEmail,
  onOpenAttachment,
  emailRefAllowlist,
  draftRefAllowlist,
}: {
  content: string;
  sources: ChatMessageSource[];
  accountId: string;
  onOpenEmail?: () => void;
  onOpenAttachment?: (ns: 'meta' | 'attach', id: string) => void;
  /** Email IDs the chat-turn tools handed to the LLM. The `email://` link
   *  handler renders only ids in this set as `EmailRefPill`s — anything
   *  outside it is treated as a hallucinated reference: dropped to plain
   *  text and a warning logged via `useLogStore`. Pass `undefined`/missing
   *  on pre-migration messages and user/system rows; both degrade to "no
   *  pills, no warning" since absence == empty allowlist. */
  emailRefAllowlist?: string[];
  /** Same shape as `emailRefAllowlist` but for `draft://DRAFT_ID` links. */
  draftRefAllowlist?: string[];
}) {
  const byNumber = new Map<number, ChatMessageSource>();
  for (const s of sources) byNumber.set(s.citationNumber, s);
  // Set, not Array, so the link handlers' lookups are O(1).
  const emailAllowSet = new Set(emailRefAllowlist ?? []);
  const draftAllowSet = new Set(draftRefAllowlist ?? []);
  const addLog = useLogStore((s) => s.addLog);

  // Dropped-ref warnings are gathered during render and flushed from an
  // effect — never logged inline in the `a` renderer. The renderer runs in
  // React's render phase, where side effects are forbidden: a streaming
  // bubble re-renders on every token, so an inline `addLog` re-fires the same
  // "Dropping draft://…" warning dozens of times (the v0.5.x duplicate-warning
  // bug). `pendingWarnings` collects this render's messages (reset each
  // render); `loggedWarnings` is the persistent set of everything already
  // logged for this mounted bubble, so re-renders and repeated ids never
  // re-warn.
  const pendingWarnings = useRef<string[]>([]);
  pendingWarnings.current = [];
  const loggedWarnings = useRef<Set<string>>(new Set());
  useEffect(() => {
    for (const msg of pendingWarnings.current) {
      if (!loggedWarnings.current.has(msg)) {
        loggedWarnings.current.add(msg);
        addLog('warn', 'chat', msg);
      }
    }
  });

  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      urlTransform={chatUrlTransform}
      components={{
        // Custom link renderer: intercept citation:// and attachment:// hrefs
        a({ href, children }) {
          if (href?.startsWith('citation://')) {
            const n = Number(href.slice('citation://'.length));
            const source = byNumber.get(n);
            if (source) {
              return <CitationPill source={source} accountId={accountId} onOpenEmail={onOpenEmail} />;
            }
            return <>[{n}]</>;
          }
          if (href?.startsWith('attachment://')) {
            const rest = href.slice('attachment://'.length);
            const slash = rest.indexOf('/');
            if (slash !== -1) {
              const ns = rest.slice(0, slash) as 'meta' | 'attach';
              const id = rest.slice(slash + 1);
              const label = typeof children === 'string' ? children : String(children);
              return <AttachmentChip label={label} onOpen={() => onOpenAttachment?.(ns, id)} />;
            }
          }
          if (href?.startsWith('email://')) {
            const emailId = href.slice('email://'.length).trim();
            const label = typeof children === 'string' ? children : String(children);
            // Validate against the structural allowlist produced by the
            // turn's tool calls. The LLM is told (in CHAT_SYSTEM's EMAIL
            // LINKS clause) to only emit ids from tool output, but a 4B
            // local model will occasionally invent one — render plain text
            // and log a warning so the regression is visible without
            // surfacing a broken chip to the user.
            if (!emailId || !emailAllowSet.has(emailId)) {
              pendingWarnings.current.push(
                `Dropping email://${emailId || '<empty>'} — not in this turn's tool allowlist (likely hallucinated).`,
              );
              return <>{children}</>;
            }
            return <EmailRefPill emailId={emailId} accountId={accountId} label={label} onOpenEmail={onOpenEmail} />;
          }
          if (href?.startsWith('draft://')) {
            const draftId = href.slice('draft://'.length).trim();
            const label = typeof children === 'string' ? children : String(children);
            // Same allowlist guarantee as `email://` — drafts the LLM never
            // saw can't be opened via a chip.
            if (!draftId || !draftAllowSet.has(draftId)) {
              pendingWarnings.current.push(
                `Dropping draft://${draftId || '<empty>'} — not in this turn's tool allowlist (likely hallucinated).`,
              );
              return <>{children}</>;
            }
            return <DraftRefPill draftId={draftId} accountId={accountId} label={label} onOpenEmail={onOpenEmail} />;
          }
          // Any remaining non-http(s)/mailto scheme is treated as a stray
          // model invention and rendered as plain text — keeps the bubble
          // from launching useless browser tabs against custom schemes.
          const isSafeExternal = !!href && /^(https?:|mailto:)/i.test(href);
          if (!isSafeExternal) {
            return <>{children}</>;
          }
          return (
            <a
              href={href}
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary-600 underline hover:text-primary-800"
            >
              {children}
            </a>
          );
        },
        // Table styling
        table({ children }) {
          return (
            <div className="overflow-x-auto my-2">
              <table className="text-xs border-collapse w-full">{children}</table>
            </div>
          );
        },
        thead({ children }) {
          return <thead className="bg-gray-200">{children}</thead>;
        },
        th({ children }) {
          return <th className="border border-gray-300 px-2 py-1 font-semibold text-left">{children}</th>;
        },
        td({ children }) {
          return <td className="border border-gray-300 px-2 py-1">{children}</td>;
        },
        tr({ children }) {
          return <tr className="even:bg-gray-50">{children}</tr>;
        },
        // Code blocks
        code({ className, children, ...props }) {
          const isBlock = className?.startsWith('language-');
          if (isBlock) {
            return (
              <pre className="my-1.5 p-2 rounded bg-gray-800 text-gray-100 text-[11px] overflow-x-auto whitespace-pre">
                <code>{children}</code>
              </pre>
            );
          }
          return (
            <code className="px-1 py-0.5 rounded bg-gray-200 text-gray-800 text-[11px] font-mono" {...props}>
              {children}
            </code>
          );
        },
        // Lists. `list-inside` puts the marker in the same line box as
        // the item content, but only if the content isn't wrapped in a
        // block-level element. remark renders multi-line / blank-separated
        // items "loose" — `<li><p>…</p></li>` — and our `<p>` renderer is
        // block-level, so the marker ("1.") lands on one line and the
        // chip on the next. `li()` below unwraps that single-`<p>` child
        // so the marker, the pill, and trailing prose share one line.
        ul({ children }) {
          return <ul className="list-disc list-inside my-1 space-y-0.5">{children}</ul>;
        },
        ol({ children }) {
          return <ol className="list-decimal list-inside my-1 space-y-0.5">{children}</ol>;
        },
        li({ children }) {
          return <li>{unwrapLooseListParagraph(children)}</li>;
        },
        // Headings
        h1({ children }) {
          return <h1 className="text-base font-bold mt-2 mb-1">{children}</h1>;
        },
        h2({ children }) {
          return <h2 className="text-sm font-bold mt-2 mb-1">{children}</h2>;
        },
        h3({ children }) {
          return <h3 className="text-sm font-semibold mt-1.5 mb-0.5">{children}</h3>;
        },
        // Paragraphs — no extra margin so bubble stays compact. Routed
        // through the named `Paragraph` function so `<li>` can detect and
        // unwrap loose-list `<p>` children by element identity.
        p: Paragraph,
        // Blockquote
        blockquote({ children }) {
          return (
            <blockquote className="border-l-2 border-gray-300 pl-3 my-1 text-gray-600 italic">{children}</blockquote>
          );
        },
      }}
    >
      {preprocessContent(content)}
    </ReactMarkdown>
  );
}
