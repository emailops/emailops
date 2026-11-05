# Frontend (src/) — React + TypeScript

Instructions specific to the React frontend. The root `CLAUDE.md` covers
project-wide architecture, the shared logging contract and the security
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
- TypeScript types for backend structs are **generated from Rust** (via `ts-rs`), not hand-maintained. Editing a Rust struct without regenerating the TS type is a bug, not a chore.

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
- Use `addLog(level, source, message)` from `useLogStore` for UI-triggered operations (e.g., sync start/complete, embedding generation, filter refresh). Levels and sources are the shared contract in the root `CLAUDE.md`.
- Backend-initiated events arrive via the Tauri `app-log` event — subscribe once at the app root and pipe into `useLogStore`.

## Testing
- Component and hook tests run in vitest + jsdom, rendering with `createRoot` inside `act` (see `AiSettings.scroll.test.tsx`).
- End-to-end checks drive the real app over WebDriver (`verify-emailops` skill, `make verify`).
- Zustand stores: test the pure reducers and selectors directly — no React render needed.

## Lessons Learned

### Zustand Store Subscriptions
- `useStore.getState().someField` in a `useMemo` dependency array does NOT subscribe to changes — React never re-renders when that field updates. Always destructure reactive fields from the hook: `const { someField } = useStore()` and use them as memo deps.
- When a Zustand action is captured in a `useEffect` closure, it may hold a stale reference. Use `useRef` for callbacks that need the latest version inside long-lived effects (e.g., sync effects that call `refetchEmails` — the ref ensures the current `activeFilter` is included).

### Component Identity & Remounts
- **Never define a component inside another component.** It remounts the whole subtree on every parent render and silently destroys what React does not own: scroll position, focus and caret, transitions, a resized `<textarea>`. Biome enforces it (`noNestedComponentDefinitions`); `AiSettings.scroll.test.tsx` and `ClassificationSettings.scroll.test.tsx` guard the two panels it broke. Background: `docs/FRONTEND-LESSONS.md`.

### Effect Dependencies & Race Conditions
- Effects that depend on async-loaded data (e.g., `accounts`) must include the loading state in deps. An effect running before data loads will read `undefined` and silently do the wrong thing (e.g., treating a not-yet-loaded account as disabled).
- When multiple state changes trigger the same effect (filter change + account change), use `currentFetchId` / abort patterns to cancel stale fetches. Always check the ID after `await` before updating state.
- Clear stale emails (set `emails: []`) when switching filters/search so the user sees a loading state instead of mismatched results.

### Email HTML Rendering
Background and the incidents behind these rules: `docs/FRONTEND-LESSONS.md`.
- Email HTML renders in a sandboxed `<iframe srcdoc>` (`sandbox="allow-scripts"` only — no `allow-same-origin`, no `allow-popups`) via `src/components/shared/EmailHtmlFrame.tsx`. A trusted bridge script injected into the srcdoc handles auto-height, link interception (the parent routes every link through `getSafeExternalUrl` + confirmation modal + `plugin-shell.open`) and search highlighting.
- **Auto-height measures `document.body.scrollHeight`, never `document.documentElement.scrollHeight`** — the root floors to the viewport and ratchets upward. `EmailHtmlFrame.height.test.ts` guards it.
- **Never loosen the CSP to make something work** (`"csp": null`, `'unsafe-inline'` in `script-src`). The srcdoc inherits the app CSP; the only acceptable fix for a required inline script is its **sha256 hash** in the base `src-tauri/tauri.conf.json` `script-src`. Changing `BRIDGE_SCRIPT` changes the hash — `EmailHtmlFrame.csp.test.ts` names the value. Dev (Vite) relaxes the CSP, so reproduce WebView rendering bugs in a release build.
- The frame is pinned to the light scheme (`color-scheme: light only`, the bridge's `dropDarkSchemeRules`). On the app's dark theme it follows the pure planner `planEmailScheme` (`src/lib/emailScheme.ts`), which reads the message as received (`schemeSource`). `EmailHtmlFrame.colorScheme.test.ts` and `EmailHtmlFrame.darkTheme.test.tsx` cover it.
- `sanitizeEmailHtml` / `sanitizeEmailHtmlFull` keep `<style>` (`ADD_TAGS: ['style']`, `FORCE_BODY: true`) and most inline styles, but reject `behavior`/`-ms-behavior`, `expression(...)`, `javascript:` and `url(data:image/svg+xml…)`/`url(data:text/html…)`; declarations are split with a paren-aware splitter.
- **Remote content gating:** with `allowRemoteContent` false, remote `src`/`poster`/`srcset` are stripped from every tag in `REMOTE_FETCHING_TAGS`, remote `url(...)` in inline styles is rejected, the `background` attribute is stripped, and `EmailHtmlFrame` stamps a per-frame `img-src data: blob: cid:` CSP that covers `<style>` blocks. **When allowing a new tag, add it to `REMOTE_FETCHING_TAGS` if it can fetch a URL.** `EmailHtmlFrame.remoteContent.test.tsx` covers it.
- Inline `cid:` images are extracted during sync and converted to `data:` URIs. Plain-text bodies convert Outlook's `[image_url]<link>` to `<img>`/`<a>` and use `white-space:pre-wrap`, not `<pre>`.

### Dark Theme
- The dark theme is one class, `html.dark`, set by `useThemeStore` from the `ui_theme` preference (`src/lib/theme.ts`). `src/index.css` remaps the Tailwind gray scale, `bg-white` and the pale accent tints under it, so components keep their light-theme classes — **do not add `dark:` twins to ordinary utilities**. See the "Dark theme" comment in `index.css` and the 2026-10-17 entry in `docs/DECISIONS.md`.
- Every screen is written in the light idiom (`bg-white`, `text-gray-900`, `border-gray-200`) and shares the inbox's surfaces: white on light, navy on dark — Lenses, EO Docs, Skills, dashboard, onboarding, lock screen and toasts included. The sidebar, the log panel (the bottom output bar and its log) and the dialogs carry `theme-chrome` on their root (the sidebar `<aside>`, `LogPanel`'s root, `common/Modal.tsx`'s shell, the settings, account and IMAP dialogs, `AiWorkInProgressDialog`): it applies the dark palette inside them on the light theme too, so they stay dark there and match everything on the dark theme. A new dialog that does not use `Modal` needs the class on its panel.
- On light surfaces use the **gray** scale and the accent steps 50–200 (tints) / 600–900 (text); `slate`/`zinc`/`neutral` are not remapped and will stay light. Never paint a surface with an inline `style` colour or an arbitrary `bg-[#…]` — a class is what the remap reaches (the virtual list's row wrapper was the one case).
- Email bodies are the one surface the remap does not reach (they live in an iframe); see "Email HTML Rendering" above for how they follow the dark theme.
- Dropdowns go through `shared/Select.tsx` (`LanguageSelect` for the UI language), never a raw `<select>`: WebKitGTK paints a native popup light whatever the theme, so on Linux the component owns the popup; macOS and Windows keep the native control. `Select.nativeGuard.test.ts` enforces it; `testId` puts a `data-testid` on whichever element is the control.
- `src/lib/darkPalette.test.ts` fails when a component uses a pale accent class (`bg-teal-50`, `text-orange-700`, `bg-red-50/40`…) that `index.css` has no override for; add the colour to the generated block rather than skipping the test. It also fails on `theme-pinned`, an arbitrary hex colour class or a `neutral`/`zinc`/`slate`/`stone` class in a component — anything the remap cannot reach — and when a dark override in `index.css` does not also cover `.theme-chrome`.

### Tauri 2 Specifics (frontend-visible)
- `@tauri-apps/plugin-shell` `open()` requires a capabilities file at `src-tauri/capabilities/default.json` with `"shell:allow-open"`. Without it, `open()` silently fails with no error.
- Tauri command parameter names auto-convert between JS camelCase and Rust snake_case. No `#[serde(rename)]` needed on command params.
