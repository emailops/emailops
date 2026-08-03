# Frontend (src/) — React + TypeScript

Instructions specific to the React frontend. The root `CLAUDE.md` covers
project-wide architecture, Rust/backend conventions, database, and security
guardrails — read it first.

## Project Layout

```
src/
├── components/   # React components (PascalCase.tsx)
├── hooks/        # Custom React hooks (use*.ts)
├── stores/       # Zustand stores
├── lib/          # Utilities and Tauri bindings (api.ts)
├── types/        # TypeScript types (mostly generated from Rust)
├── App.tsx
└── main.tsx
```

## Coding Standards

### Naming Conventions
- Components: `PascalCase` (e.g., `EmailList.tsx`)
- Hooks: `camelCase` with `use` prefix (e.g., `useEmails.ts`)
- Utilities: `camelCase` (e.g., `formatDate.ts`)
- Types/Interfaces: `PascalCase` (e.g., `Email`, `AccountConfig`)

### Component Structure
```typescript
// Prefer function components with explicit return types
interface EmailListProps {
  contextId: string | null;
  onSelectEmail: (email: Email) => void;
}

export function EmailList({ contextId, onSelectEmail }: EmailListProps) {
  // hooks first
  const { emails, isLoading } = useEmails(contextId);

  // early returns for loading/error states
  if (isLoading) return <Spinner />;

  // main render
  return (
    <div className="email-list">
      {emails.map(email => (
        <EmailRow key={email.id} email={email} onClick={onSelectEmail} />
      ))}
    </div>
  );
}
```

### Tauri API Calls — centralized invoke + generated types
- All Tauri `invoke(…)` calls live in `src/lib/api.ts`. Components and hooks import typed wrappers from there. A lint rule (`no-invoke-outside-api`) enforces this — do not bypass it.
- TypeScript types for backend structs are **generated from Rust** (via `specta` / `ts-rs`), not hand-maintained. Editing a Rust struct without regenerating the TS type is a bug, not a chore.

```typescript
// src/lib/api.ts
import { invoke } from '@tauri-apps/api/core';

export async function getEmails(contextId?: string): Promise<Email[]> {
  return invoke('get_emails', { contextId });
}

// Use in components via custom hooks
export function useEmails(contextId: string | null) {
  const [emails, setEmails] = useState<Email[]>([]);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    getEmails(contextId ?? undefined)
      .then(setEmails)
      .finally(() => setIsLoading(false));
  }, [contextId]);

  return { emails, isLoading };
}
```

### State Management (Zustand) — pure reducers + colocated selectors
```typescript
// Keep stores focused and small
interface AccountStore {
  accounts: Account[];
  activeAccountId: string | null;
  setActiveAccount: (id: string) => void;
  fetchAccounts: () => Promise<void>;
}

export const useAccountStore = create<AccountStore>((set) => ({
  accounts: [],
  activeAccountId: null,
  setActiveAccount: (id) => set({ activeAccountId: id }),
  fetchAccounts: async () => {
    const accounts = await api.getAccounts();
    set({ accounts });
  },
}));
```

- State transitions live in **pure reducer functions** that take `(state, action) -> state` and are unit-testable without React.
- Async actions are thin: they perform side effects then call the pure reducer at the end.
- Selectors are exported alongside the store; tests assert on selectors, not raw state shape.
- See "Lessons Learned → Zustand Store Subscriptions" below for the stale-closure pitfalls these patterns prevent.

## Frontend Robustness
- Add a React error boundary at the app root.
- Async UI flows must guard against stale responses and must always release loading locks in cancellation/error paths.
- Validate backend event payloads and enum-like values before indexing into UI maps or assuming a type shape.
- User-facing failures must surface in the output panel or visible UI state; do not rely on `console.error` alone.

## Logging / Output Panel (frontend)
- Use `addLog(level, source, message)` from `useLogStore` for UI-triggered operations (e.g., sync start/complete, embedding generation, filter refresh).
- Levels: `info` (start), `success` (completion), `error` (failure), `debug` (verbose progress).
- Sources: `sync`, `embeddings`, `account`, `ai`, `system`.
- Backend-initiated events arrive via the Tauri `app-log` event — subscribe once at the app root and pipe into `useLogStore`.

## Testing
- Component tests with React Testing Library.
- Hook tests with @testing-library/react-hooks.
- E2E tests with Playwright (post-MVP).
- Zustand stores: test the pure reducers and selectors directly — no React render needed.

## Lessons Learned

### Zustand Store Subscriptions
- `useStore.getState().someField` in a `useMemo` dependency array does NOT subscribe to changes — React never re-renders when that field updates. Always destructure reactive fields from the hook: `const { someField } = useStore()` and use them as memo deps.
- When a Zustand action is captured in a `useEffect` closure, it may hold a stale reference. Use `useRef` for callbacks that need the latest version inside long-lived effects (e.g., sync effects that call `refetchEmails` — the ref ensures the current `activeFilter` is included).

### Effect Dependencies & Race Conditions
- Effects that depend on async-loaded data (e.g., `accounts`) must include the loading state in deps. An effect running before data loads will read `undefined` and silently do the wrong thing (e.g., treating a not-yet-loaded account as disabled).
- When multiple state changes trigger the same effect (filter change + account change), use `currentFetchId` / abort patterns to cancel stale fetches. Always check the ID after `await` before updating state.
- Clear stale emails (set `emails: []`) when switching filters/search so the user sees a loading state instead of mismatched results.

### Email HTML Rendering
- Email HTML renders inside a sandboxed `<iframe srcdoc>` (`sandbox="allow-scripts allow-popups"`, no `allow-same-origin`) via `src/components/shared/EmailHtmlFrame.tsx`. The null-origin iframe gives the email its own document, which (a) lets modern email templates work — ticket cut-outs, responsive `<style>` media queries, hero-image sizing, `display:none`/`visibility:hidden` for mobile-only blocks — without leaking CSS into the app's Tailwind layer, (b) prevents any script that somehow survives DOMPurify from touching the parent DOM, cookies, or storage, and (c) blocks the iframe from navigating the top window.
- A small trusted bridge script injected into the srcdoc handles auto-height (`ResizeObserver` → `postMessage({type:'height'})`), link interception (`postMessage({type:'link', href})` → parent routes via `getSafeExternalUrl` + confirmation modal + `plugin-shell.open`), and search highlighting (parent posts the query, iframe walks text nodes and wraps `<mark>`s).
- **Auto-height must measure `document.body.scrollHeight`, never `document.documentElement.scrollHeight`.** The root element's `scrollHeight` floors to the viewport height, and inside the frame the viewport equals the height the parent just set on the iframe — so feeding it back creates a one-way ratchet that can only grow (the v0.5.0 runaway-height bug: a ~20,654px email measured 23,622px and climbed). The body shrink-wraps its content, so its `scrollHeight` reflects the true height and stays stable across re-measurements. `EmailHtmlFrame.height.test.ts` evals the real `BRIDGE_SCRIPT` under jsdom with the two scroll heights stubbed apart and asserts the posted height is the body value — keep it green.
- **CSP differs between dev and production — never loosen or bypass it to make something work locally.** Tauri's dev server (Vite HMR) relaxes the configured CSP with `'unsafe-inline'`/dev-server allowances, so inline `<script>`/`<style>` that runs fine in `make dev` can be silently blocked in a release build. A `srcdoc` iframe inherits the parent document's CSP, so the injected bridge script is governed by the app `script-src` in `src-tauri/tauri.conf.json`. This is exactly how the v0.5.0 "body collapses to ~40px" bug happened: production CSP blocked the inline bridge, no `height` message was ever posted, and the iframe stayed at its 40px initial height. Do **not** "fix" CSP failures by setting `"csp": null`, adding `'unsafe-inline'` to `script-src`, or otherwise weakening the policy. The only acceptable fix for a required inline script is to allowlist it by its **sha256 hash** in `script-src` (base config only — `tauri.intel.conf.json` merges on top and inherits it). `EmailHtmlFrame.csp.test.ts` recomputes the hash from `BRIDGE_SCRIPT` and asserts it is present, so any edit to the bridge script forces the CSP to be updated in lockstep — keep that test green rather than deleting or skipping it. When debugging WebView rendering, always reproduce in a release/`tauri build` context (not just dev), since dev's relaxed CSP hides production-only failures.
- `sanitizeEmailHtml` / `sanitizeEmailHtmlFull` keep `<style>` tags (`ADD_TAGS: ['style']` plus `FORCE_BODY: true` so DOMPurify doesn't drop them into a stripped `<head>`) and most inline-style properties. `behavior` / `-ms-behavior` are still blocked because they're the historical CSS-as-script vector. `expression(...)`, `javascript:` URIs, and `url(data:image/svg+xml…)` / `url(data:text/html…)` remain rejected at the value level. CSS declarations are split via a paren-aware splitter so semicolons inside `url(data:image/...;base64,...)` don't bisect a single declaration into evading fragments.
- Remote content gating still applies. `sanitizeEmailHtmlFull(html, allowRemoteContent)` strips remote `src`/`poster`/`srcset` from **every** element that fetches on its own — `<img>`, `<source>`, `<video>`, `<audio>`, `<track>` (the `REMOTE_FETCHING_TAGS` set) — and rejects remote `url(...)` inside inline `style` attributes when `allowRemoteContent` is false. Gating only `<img>`/`<source>` was a real read-receipt hole: DOMPurify's `html` profile passes `<video>`/`<audio>` through, and `<video poster>` fetches on render with no user interaction. **When adding a tag to the allowed set, check whether it can fetch a URL and add it to `REMOTE_FETCHING_TAGS` if so.** CSS inside `<style>` blocks is *not* yet scanned for remote URLs — adding a CSS parser is the right next step if that becomes a privacy gap.
- Inline images use `cid:` Content-ID references to MIME attachment parts. The Gmail API returns these as base64-encoded parts with `Content-ID` headers. Extract during sync and convert to `data:` URIs in the HTML body.
- Plain text emails (no HTML part) need proper conversion: `[image_url]<link>` is Outlook's format for linked images. Convert to `<img>` / `<a>` tags. Use `white-space:pre-wrap` instead of `<pre>` for better flow.

### Tauri 2 Specifics (frontend-visible)
- `@tauri-apps/plugin-shell` `open()` requires a capabilities file at `src-tauri/capabilities/default.json` with `"shell:allow-open"`. Without it, `open()` silently fails with no error.
- Tauri command parameter names auto-convert between JS camelCase and Rust snake_case. No `#[serde(rename)]` needed on command params.
