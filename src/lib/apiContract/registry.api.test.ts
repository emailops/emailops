// Contract: nothing in api.ts calls a command that does not exist, and every
// command file is checked by one of the per-feature contract tests here.
import { describe, expect, it } from 'vitest';
import { commandFiles, invokeSites, rustCommands } from './contract';

const CHECKED_FILES = [
  'accounts.rs',
  'agent.rs',
  'ai_config.rs',
  'ai_models.rs',
  'attachments.rs',
  'calendar.rs',
  'chat.rs',
  'classification.rs',
  'connectivity.rs',
  'contacts.rs',
  'dashboard.rs',
  'drafts.rs',
  'emails.rs',
  'filters.rs',
  'junk.rs',
  'lenses.rs',
  'memory.rs',
  'outbox.rs',
  'preferences.rs',
  'prompts.rs',
  'search.rs',
  'security.rs',
  'sender_controls.rs',
  'skills.rs',
  'system.rs',
  'translation.rs',
  'trusted_senders.rs',
];

describe('api.ts ↔ Rust: command registry', () => {
  it('every command api.ts invokes is a #[tauri::command]', () => {
    const known = new Set(
      commandFiles()
        .flatMap(rustCommands)
        .map((c) => c.name),
    );
    const unknown = invokeSites()
      .map((s) => s.command)
      .filter((name) => !name.startsWith('plugin:') && !known.has(name));
    expect(unknown).toEqual([]);
  });

  it('every command file belongs to a per-feature contract test', () => {
    expect(commandFiles().sort()).toEqual(CHECKED_FILES);
  });
});
