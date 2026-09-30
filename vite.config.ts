/// <reference types="vitest" />

import react from '@vitejs/plugin-react';
import path from 'path';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(import.meta.dirname, './src'),
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    // Never pick up test files from agent worktrees under .claude/ — they
    // duplicate src/ against a different node_modules (two React copies →
    // null-hooks errors) and would fail main's suite spuriously.
    exclude: ['**/node_modules/**', '**/dist/**', '.claude/**'],
    // Only read by `vitest run --coverage` (make coverage-ts); the plain gate
    // run does not collect coverage. Reports land in the gitignored
    // src-tauri/reports/ tree next to the Rust coverage.
    coverage: {
      provider: 'v8',
      include: ['src/**/*.{ts,tsx}'],
      exclude: ['src/**/*.test.{ts,tsx}', 'src/**/*.d.ts', 'src/types/**', 'src/main.tsx'],
      reporter: ['text-summary', 'html', 'json-summary', 'json'],
      reportsDirectory: 'src-tauri/reports/coverage/ts',
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
});
