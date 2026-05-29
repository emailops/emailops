# E2E Test Plan — Tauri Driver + Playwright

## Architecture

```
Playwright (Node.js) → WebDriver protocol → tauri-driver → EmailOps app (webview)
```

- `tauri-driver` launches the built Tauri app and exposes a WebDriver endpoint
- Playwright connects via `cdp` (Chrome DevTools Protocol) or WebDriver
- Tests interact with the real app UI — full backend, real SQLite DB, real sync (or mocked)

## Prerequisites

```bash
# 1. Install tauri-driver (Rust CLI tool)
cargo install tauri-driver

# 2. Install Playwright
npm install -D @playwright/test

# 3. Build the app (tauri-driver needs the binary)
npm run tauri build -- --debug
```

## Test Database Strategy

Tests use a **separate test DB** to avoid polluting production data:
- Set `EMAILOPS_TEST_DB=1` env var → app uses `emailops_test.db` in a temp directory
- Seed test data via direct SQLite inserts before tests
- Wipe DB between test suites

Alternatively, copy the production DB as a fixture for read-only tests.

## Test Setup

```typescript
// tests/e2e/playwright.config.ts
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/e2e',
  timeout: 30_000,
  retries: 0,
  use: {
    // Connect to tauri-driver's WebDriver endpoint
    baseURL: 'http://localhost:4444',
  },
});
```

```typescript
// tests/e2e/fixtures.ts
import { test as base } from '@playwright/test';
import { spawn, ChildProcess } from 'child_process';
import path from 'path';

// Start tauri-driver + app before all tests
export const test = base.extend<{}, { tauriDriver: ChildProcess }>({
  tauriDriver: [async ({}, use) => {
    const driver = spawn('tauri-driver', [], {
      env: {
        ...process.env,
        EMAILOPS_DEV_TOKENS: '1',
        EMAILOPS_GMAIL_CLIENT_ID: process.env.EMAILOPS_GMAIL_CLIENT_ID,
        EMAILOPS_GMAIL_CLIENT_SECRET: process.env.EMAILOPS_GMAIL_CLIENT_SECRET,
      },
    });
    
    // Wait for driver to be ready
    await new Promise(resolve => setTimeout(resolve, 3000));
    
    await use(driver);
    
    driver.kill();
  }, { scope: 'worker' }],
});
```

## Test Suites

### 1. Smoke Test — App Launches
```
- App window opens
- Sidebar is visible with "EmailOps" title
- "No accounts connected" shown if fresh DB
- Log panel is present at bottom
```

### 2. Account Management
```
- Account list shows connected accounts
- Account order matches sort_order
- Hover shows up/down/enable controls
- Move up: first account goes to second position
- Move down: last account stays (button hidden)
- Disable account: shows "(disabled)" and reduced opacity
- Click disabled account: loads cached emails, no sync progress
- Re-enable account: sync triggers on next select
```

### 3. Email List
```
- Emails load after account selection
- Sender, subject, snippet visible with correct contrast
- Unread emails have blue dot and bold text
- Scroll triggers load more (infinite scroll)
- "Load more" button works
- Category checkboxes filter emails client-side
- Three-dot menu opens dropdown
- "Copy email ID" copies to clipboard
- "Add sender as smart filter" adds filter to sidebar
```

### 4. Email View
```
- Click email: thread view opens on right
- Subject shown in header
- Sender avatar, name, email, date displayed
- HTML body renders with images
- Plain text body renders with links and formatting
- Links show confirmation dialog before opening
- Thread emails: collapsed by default, click to expand
```

### 5. Smart Filters
```
- Section visible in sidebar with "Smart Filters" header
- Refresh icon triggers recalculation
- Filters show domain/sender icon + count
- Click filter: inbox filters to matching emails
- Click again: filter deactivates
- Hover: pin/remove icons appear
- Pin filter: persists across restart
- Remove filter: disappears, doesn't return on refresh
- "Add sender as smart filter" from email menu: appears in sidebar
```

### 6. Search
```
- Cmd+K opens search overlay
- Typing queries shows results
- Structured filters work: from:, to:, subject:
- AI search indicator shows when Ollama is available
- Click result: opens email in thread view
- Esc closes search
```

### 7. Sync
```
- Account select triggers sync
- Progress shows "Checking for new emails..."
- No new emails: "Inbox up to date" (fast)
- New emails: download count shown
- Disabled account: "sync skipped"
- Sync error: error banner appears with reauth option
```

## Running Tests

```bash
# Build the app first (required for tauri-driver)
npm run tauri build -- --debug

# Run all E2E tests
npx playwright test tests/e2e/

# Run a specific suite
npx playwright test tests/e2e/accounts.spec.ts

# Debug mode (headed, step-by-step)
npx playwright test --debug
```

## CI Considerations

- Requires a display server (Xvfb on Linux, native on macOS)
- Build step is slow (~3-5 min) — cache the binary
- Tests need env vars for Gmail OAuth (or use mock data only)
- Consider running only smoke + account + email list tests in CI
- Full sync tests only in nightly/manual runs
