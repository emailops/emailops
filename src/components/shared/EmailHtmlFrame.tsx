// Renders pre-sanitized email HTML inside a sandboxed `<iframe srcdoc>`.
//
// Why an iframe: modern email templates rely on `<style>` blocks (responsive
// media queries, ticket cut-outs, hero-image sizing). Rendering them in a
// regular div forces a choice between stripping `<style>` (breaks layout) and
// letting it cascade into the app's Tailwind (breaks the app). The iframe
// sandbox (no `allow-same-origin`, no `allow-top-navigation`) gives the email
// a null origin so its CSS can't escape and its scripts (already stripped by
// DOMPurify) couldn't reach the parent DOM even if they survived.
//
// The bridge script we inject is the *only* trusted JS that runs inside the
// frame. It handles three things via postMessage:
//   - auto-height (ResizeObserver on documentElement → `height` message)
//   - link click interception (capture-phase listener → `link` message; parent
//     decides whether to open via Tauri shell after a confirmation modal)
//   - in-frame search highlighting (wraps text matches in `<mark>` and scrolls
//     the first hit into view)

import { open } from '@tauri-apps/plugin-shell';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getSafeExternalUrl, type ParsedMailto, parseMailtoUrl } from '@/lib/emailFormatting';
import {
  declaresOwnColors,
  type EmailBodyTheme,
  type EmailThemeOverride,
  emailThemeCss,
  planEmailBodyTheme,
} from '@/lib/emailTheme';
import { computeMatchScrollTop, findScrollParent } from '@/lib/matchScroll';
import { useThemeStore } from '@/stores/themeStore';

export interface EmailHtmlFrameProps {
  /** Force this one message light or dark, overriding the app theme. Null
   *  follows the app. Inversion is lossy, so the reader needs a way out. */
  themeOverride?: EmailThemeOverride;
  /** Already-sanitized HTML. Callers run `sanitizeEmailHtml(Full)` themselves. */
  html: string;
  /** Optional substring to highlight in the rendered body. Case-insensitive. */
  highlightQuery?: string | null;
  /** Index of the occurrence inside this body that is the active search
   *  match. It gets distinct styling and the surrounding scroll container is
   *  scrolled to it. null/undefined → no active occurrence in this body. */
  activeMatchIndex?: number | null;
  /** Reports how many occurrences of `highlightQuery` this body contains,
   *  whenever the highlight is (re-)applied. */
  onMatchesReported?: (count: number) => void;
  /** Class applied to the iframe element. */
  className?: string;
  /** Called when the user clicks a mailto: link in the body. When omitted,
   *  mailto clicks are ignored (they are never sent to the OS handler). */
  onMailtoLink?: (mailto: ParsedMailto) => void;
}

// Defined as a string so we can stamp it into srcDoc. Lives in `<head>` so
// stray `</script>` text in email body content (e.g. inside `<style>` blocks) // i18n-ignore: source-code comment, not user-facing
// cannot terminate it — head parses before body.
export const BRIDGE_SCRIPT = String.raw`
(function(){
  var zoomLevel = 1;
  var gestureBaseZoom = 1;
  function send(msg){
    parent.postMessage(Object.assign({ __emailFrame: true }, msg), '*');
  }
  function postHeight(){
    // Measure the body's content height, NOT documentElement.scrollHeight.
    // The root element's scrollHeight floors to the viewport height, and the
    // viewport equals the height the parent just set on the iframe — using it
    // creates a one-way ratchet that can only grow (the v0.5.0 runaway-height
    // bug). The body shrink-wraps its content, so its scrollHeight reflects the
    // real height and stays stable across re-measurements.
    //
    // Zoom lives on the documentElement, so the body's scrollHeight stays in
    // its own (unzoomed) coordinate space — multiply by the zoom factor to get
    // the visual height the parent must give the iframe.
    var h = document.body ? document.body.scrollHeight : document.documentElement.scrollHeight;
    send({ type: 'height', height: Math.round(h * zoomLevel) });
  }
  function applyZoom(z){
    z = Math.min(3, Math.max(0.5, z));
    zoomLevel = z;
    var root = document.documentElement;
    root.style.zoom = z === 1 ? '' : String(z);
    root.setAttribute('data-email-zoom', String(z));
    postHeight();
  }
  function init(){
    // macOS touchpad pinch: Chromium/Firefox deliver it as ctrl+wheel; WebKit
    // (the Tauri webview) fires proprietary gesture events with an absolute
    // scale relative to the gesture's start. Cmd/Ctrl+0 resets.
    window.addEventListener('wheel', function(e){
      if (!e.ctrlKey) return;
      e.preventDefault();
      applyZoom(zoomLevel * Math.exp(-e.deltaY * 0.01));
    }, { passive: false });
    window.addEventListener('gesturestart', function(e){
      e.preventDefault();
      gestureBaseZoom = zoomLevel;
    });
    window.addEventListener('gesturechange', function(e){
      e.preventDefault();
      if (e.scale) applyZoom(gestureBaseZoom * e.scale);
    });
    window.addEventListener('gestureend', function(e){
      e.preventDefault();
    });
    window.addEventListener('keydown', function(e){
      if ((e.metaKey || e.ctrlKey) && e.key === '0'){
        e.preventDefault();
        applyZoom(1);
      }
    });
    if (window.ResizeObserver){
      var ro = new ResizeObserver(postHeight);
      ro.observe(document.documentElement);
      if (document.body) ro.observe(document.body);
    }
    // Late image loads change height after the initial load event.
    document.querySelectorAll('img').forEach(function(img){
      if (!img.complete) img.addEventListener('load', postHeight);
      img.addEventListener('error', postHeight);
    });
    document.addEventListener('click', function(e){
      var t = e.target;
      var a = t && t.closest ? t.closest('a') : null;
      if (!a) return;
      var href = a.getAttribute('href');
      if (!href) return;
      e.preventDefault();
      send({ type: 'link', href: href });
    }, true);
    window.addEventListener('message', function(ev){
      var data = ev.data || {};
      if (data && data.__emailFrameCmd === 'highlight'){
        clearMarks();
        var marks = wrapMatches(String(data.query || ''));
        // The parent tells us which occurrence inside THIS body is the
        // globally active one; style it distinctly and report its position so
        // the parent can scroll its container (a null-origin iframe cannot
        // scroll the parent itself).
        var activeIndex = typeof data.activeIndex === 'number' ? data.activeIndex : -1;
        var activeTop = null;
        if (activeIndex >= 0 && activeIndex < marks.length){
          var active = marks[activeIndex];
          active.setAttribute('data-email-search-active', '1');
          active.style.backgroundColor = '#f59e0b';
          activeTop = active.getBoundingClientRect().top;
        }
        send({ type: 'matches', count: marks.length, activeTop: activeTop });
        postHeight();
      }
    });
    postHeight();
    window.addEventListener('load', postHeight);
  }
  function clearMarks(){
    var marks = document.querySelectorAll('mark[data-email-search-mark]');
    marks.forEach(function(m){
      var p = m.parentNode;
      if (!p) return;
      while (m.firstChild) p.insertBefore(m.firstChild, m);
      p.removeChild(m);
      if (p.normalize) p.normalize();
    });
  }
  function wrapMatches(q){
    if (!q) return [];
    var ql = q.toLowerCase();
    var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
      acceptNode: function(n){
        var p = n.parentElement;
        if (!p) return NodeFilter.FILTER_REJECT;
        var t = p.tagName;
        if (t === 'SCRIPT' || t === 'STYLE' || t === 'MARK') return NodeFilter.FILTER_REJECT;
        if (!n.nodeValue || n.nodeValue.toLowerCase().indexOf(ql) === -1) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      }
    });
    var nodes = []; var n;
    while ((n = walker.nextNode())) nodes.push(n);
    var allMarks = [];
    nodes.forEach(function(textNode){
      var text = textNode.nodeValue || '';
      var lower = text.toLowerCase();
      var frag = document.createDocumentFragment();
      var cursor = 0;
      while (cursor < text.length){
        var idx = lower.indexOf(ql, cursor);
        if (idx === -1){
          frag.appendChild(document.createTextNode(text.slice(cursor)));
          break;
        }
        if (idx > cursor) frag.appendChild(document.createTextNode(text.slice(cursor, idx)));
        var mark = document.createElement('mark');
        mark.setAttribute('data-email-search-mark', '1');
        mark.style.backgroundColor = '#fde68a';
        mark.style.color = 'inherit';
        mark.style.padding = '0 1px';
        mark.style.borderRadius = '2px';
        mark.textContent = text.slice(idx, idx + q.length);
        allMarks.push(mark);
        frag.appendChild(mark);
        cursor = idx + q.length;
      }
      if (textNode.parentNode) textNode.parentNode.replaceChild(frag, textNode);
    });
    return allMarks;
  }
  if (document.readyState === 'loading'){
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
`;

// Baseline styles inside the frame. Tailwind doesn't apply here — these
// reproduce the look the previous div-based renderer had via utility classes.
export const FRAME_BASE_CSS = `
  /* Opaque white in BOTH app themes — see EmailHtmlFrame.theme.test.ts. The
     email carries the sender's CSS, and most mail sets a dark text colour
     without setting a background, so anything but a light card renders the
     message invisible under dark mode. */
  html, body { margin: 0; padding: 0; background: #ffffff; }
  body {
    /* flow-root establishes a block formatting context so the first/last child's
       vertical margins stay *inside* the body instead of collapsing through it.
       With a margin-less body, a leading <p>'s top margin escapes above the body
       and shifts content down without being counted in body.scrollHeight — so
       auto-height under-measures by that margin and clips the trailing footer.
       Containing the margins keeps scrollHeight accurate. */
    display: flow-root;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
    color: #1f2937;
    font-size: 14px;
    line-height: 1.5;
    word-wrap: break-word;
    overflow-wrap: anywhere;
  }
  img { max-width: 100%; height: auto; }
  a { color: #2563eb; text-decoration: underline; cursor: pointer; }
  table { max-width: 100%; }
  blockquote {
    border-left: 4px solid #d1d5db;
    padding-left: 1rem;
    color: #4b5563;
    margin: 0.5rem 0;
  }
  pre { white-space: pre-wrap; word-break: break-word; }
`;

function buildSrcDoc(sanitizedHtml: string, bodyTheme: EmailBodyTheme): string {
  // The theme stylesheet comes AFTER the base one so it wins on equal
  // specificity, and after nothing else — the email's own <style> lives in the
  // body and still overrides both, which is the point: its palette is
  // deliberate, and `dark-inverted` transforms it rather than fighting it.
  return `<!doctype html><html><head>
<meta charset="utf-8">
<base target="_top">
<style>${FRAME_BASE_CSS}${emailThemeCss(bodyTheme)}</style>
<script>${BRIDGE_SCRIPT}</script>
</head><body>${sanitizedHtml}</body></html>`;
}

export function EmailHtmlFrame({
  html,
  highlightQuery,
  activeMatchIndex,
  onMatchesReported,
  className,
  onMailtoLink,
  themeOverride = null,
}: EmailHtmlFrameProps) {
  const { t } = useTranslation(['common', 'inbox']);
  const frameRef = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(40);
  const [confirmUrl, setConfirmUrl] = useState<string | null>(null);

  const appTheme = useThemeStore((s) => s.theme);
  const bodyTheme = useMemo(
    () => planEmailBodyTheme({ appTheme, override: themeOverride, declaresColors: declaresOwnColors(html) }),
    [appTheme, themeOverride, html],
  );
  const srcDoc = useMemo(() => buildSrcDoc(html, bodyTheme), [html, bodyTheme]);

  useEffect(() => {
    function onMessage(e: MessageEvent) {
      const data = e.data as {
        __emailFrame?: boolean;
        type?: string;
        height?: number;
        href?: string;
        count?: number;
        activeTop?: number | null;
      } | null;
      if (!data?.__emailFrame) return;
      if (e.source !== frameRef.current?.contentWindow) return;
      if (data.type === 'height' && typeof data.height === 'number') {
        // Clamp to avoid runaway growth from rounding loops; emails over
        // 50k px tall are pathological and almost always tracking artefacts.
        const clamped = Math.min(Math.max(data.height, 40), 50000);
        setHeight((prev) => (Math.abs(prev - clamped) > 1 ? clamped : prev));
      } else if (data.type === 'matches') {
        if (typeof data.count === 'number') onMatchesReported?.(data.count);
        // The bridge only reports a position when this body holds the active
        // occurrence; scroll the surrounding container to it — the sandboxed
        // iframe can't scroll the parent itself.
        if (typeof data.activeTop === 'number' && frameRef.current) {
          const container = findScrollParent(frameRef.current);
          if (container) {
            container.scrollTo({
              top: computeMatchScrollTop(
                {
                  scrollTop: container.scrollTop,
                  rectTop: container.getBoundingClientRect().top,
                  clientHeight: container.clientHeight,
                },
                frameRef.current.getBoundingClientRect().top,
                data.activeTop,
              ),
              behavior: 'smooth',
            });
          }
        }
      } else if (data.type === 'link' && typeof data.href === 'string') {
        const href = data.href;
        if (href.startsWith('#')) return;
        const mailto = parseMailtoUrl(href);
        if (mailto) {
          onMailtoLink?.(mailto);
          return;
        }
        if (href.toLowerCase().startsWith('mailto:')) return;
        const safe = getSafeExternalUrl(href);
        if (!safe) return;
        setConfirmUrl(safe);
      }
    }
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  }, [onMailtoLink, onMatchesReported]);

  // Re-send the highlight command whenever the query or the frame contents
  // change. `srcDoc` is intentional in the dep list — when html changes the
  // iframe reloads and previously-applied highlights are wiped, so we must
  // re-apply once the new body has parsed (the `onLoad` handler covers the
  // immediate post-reload case; this effect covers query updates after that).
  // biome-ignore lint/correctness/useExhaustiveDependencies: srcDoc presence is intentional
  useEffect(() => {
    const win = frameRef.current?.contentWindow;
    if (!win) return;
    win.postMessage(
      { __emailFrameCmd: 'highlight', query: highlightQuery ?? '', activeIndex: activeMatchIndex ?? null },
      '*',
    );
  }, [highlightQuery, activeMatchIndex, srcDoc]);

  const handleLoad = useCallback(() => {
    const win = frameRef.current?.contentWindow;
    if (!win) return;
    win.postMessage(
      { __emailFrameCmd: 'highlight', query: highlightQuery ?? '', activeIndex: activeMatchIndex ?? null },
      '*',
    );
  }, [highlightQuery, activeMatchIndex]);

  const handleConfirmOpen = useCallback(async () => {
    if (!confirmUrl) return;
    try {
      await open(confirmUrl);
    } catch (err) {
      console.error('Failed to open URL:', err);
    }
    setConfirmUrl(null);
  }, [confirmUrl]);

  return (
    <>
      <iframe
        ref={frameRef}
        title={t('inbox:emailView.frameTitle')}
        sandbox="allow-scripts allow-popups"
        srcDoc={srcDoc}
        onLoad={handleLoad}
        className={className}
        style={{ width: '100%', height, border: 'none', display: 'block' }}
      />
      {confirmUrl && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white rounded-xl shadow-xl p-6 max-w-md w-full mx-4 dark:bg-surface">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
              {t('inbox:emailView.openLinkTitle')}
            </h3>
            <p className="mt-2 text-sm text-gray-600 break-all dark:text-gray-400">{confirmUrl}</p>
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setConfirmUrl(null)}
                className="px-4 py-2 text-sm font-medium text-gray-700 bg-gray-100 hover:bg-gray-200 rounded-lg transition-colors dark:text-gray-300 dark:bg-surface-hover dark:hover:bg-gray-700"
              >
                {t('common:actions.cancel')}
              </button>
              <button
                type="button"
                onClick={handleConfirmOpen}
                className="px-4 py-2 text-sm font-medium text-white bg-primary-600 hover:bg-primary-700 rounded-lg transition-colors"
              >
                {t('inbox:emailView.openInBrowser')}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
