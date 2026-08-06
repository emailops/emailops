import { describe, expect, it } from 'vitest';
import {
  buildFeedbackEmail,
  FEEDBACK_RECIPIENT,
  FEEDBACK_TYPES,
  type FeedbackTech,
  formatFeedbackTech,
  friendlyOsName,
} from './feedback';

const baseTech: FeedbackTech = {
  appVersion: '0.6.0',
  osPlatform: 'macos',
  osVersion: '14.5.0',
  arch: 'aarch64',
  aiProvider: 'llamacpp',
  aiModel: 'qwen3.5-4b',
};

describe('friendlyOsName', () => {
  it('maps known Tauri platform codes to display names', () => {
    expect(friendlyOsName('macos')).toBe('macOS');
    expect(friendlyOsName('windows')).toBe('Windows');
    expect(friendlyOsName('linux')).toBe('Linux');
    expect(friendlyOsName('ios')).toBe('iOS');
    expect(friendlyOsName('android')).toBe('Android');
  });

  it('capitalizes unknown platforms rather than dropping them', () => {
    expect(friendlyOsName('freebsd')).toBe('Freebsd');
  });

  it('falls back to a placeholder when the platform is empty', () => {
    expect(friendlyOsName('')).toBe('Unknown');
  });
});

describe('formatFeedbackTech', () => {
  it('assembles name, version and arch into one os string', () => {
    expect(formatFeedbackTech(baseTech).os).toBe('macOS 14.5.0 (aarch64)');
  });

  it('omits the version segment when the OS version is unknown', () => {
    expect(formatFeedbackTech({ ...baseTech, osVersion: '' }).os).toBe('macOS (aarch64)');
  });

  it('omits the arch segment when the arch is unknown', () => {
    expect(formatFeedbackTech({ ...baseTech, arch: '' }).os).toBe('macOS 14.5.0');
  });

  it('joins provider and model with a slash', () => {
    expect(formatFeedbackTech(baseTech).aiProvider).toBe('llamacpp / qwen3.5-4b');
  });

  it('shows the provider alone when there is no model', () => {
    expect(formatFeedbackTech({ ...baseTech, aiModel: '' }).aiProvider).toBe('llamacpp');
  });

  it('passes the app version straight through', () => {
    expect(formatFeedbackTech(baseTech).version).toBe('0.6.0');
  });

  it('flags a Rosetta-translated process so x86_64 is not read as an Intel Mac', () => {
    // `arch` is compile-time, so an x86_64 build reports x86_64 on a real Intel
    // Mac and on Apple Silicon under Rosetta alike — and only the former cannot
    // run the embedded AI runtime. A report has to say which one it was.
    expect(formatFeedbackTech({ ...baseTech, arch: 'x86_64', translated: true }).os).toBe(
      'macOS 14.5.0 (x86_64, Rosetta)',
    );
  });

  it('leaves the arch untouched when the process is native', () => {
    expect(formatFeedbackTech({ ...baseTech, arch: 'x86_64', translated: false }).os).toBe('macOS 14.5.0 (x86_64)');
    // Absent (older callers / a failed probe) must read the same as native.
    expect(formatFeedbackTech({ ...baseTech, arch: 'x86_64' }).os).toBe('macOS 14.5.0 (x86_64)');
  });
});

describe('buildFeedbackEmail', () => {
  // Fake translator: echoes the key, appending interpolation options when present,
  // so we can assert both the selected key and the values fed into it.
  const fakeT = (key: string, options?: Record<string, string>) =>
    options ? `${key}::${JSON.stringify(options)}` : key;

  it('always addresses the feedback mailbox', () => {
    for (const type of FEEDBACK_TYPES) {
      expect(buildFeedbackEmail(type, fakeT, baseTech).to).toBe(FEEDBACK_RECIPIENT);
    }
    expect(FEEDBACK_RECIPIENT).toBe('hello@getemailops.com');
  });

  it('selects subject and body keys per feedback type', () => {
    const bug = buildFeedbackEmail('bug', fakeT, baseTech);
    expect(bug.subject).toBe('compose:feedback.bug.subject');
    expect(bug.body).toContain('compose:feedback.bug.body');

    const idea = buildFeedbackEmail('idea', fakeT, baseTech);
    expect(idea.subject).toBe('compose:feedback.idea.subject');
  });

  it('interpolates the formatted tech info into the body', () => {
    const { body } = buildFeedbackEmail('general', fakeT, baseTech);
    expect(body).toContain('"version":"0.6.0"');
    expect(body).toContain('"os":"macOS 14.5.0 (aarch64)"');
    expect(body).toContain('"aiProvider":"llamacpp / qwen3.5-4b"');
  });
});
