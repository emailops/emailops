// Pure logic for the "Give feedback" flow: turning runtime facts + the current
// UI language into a ready-to-send feedback email (recipient, subject, body).
// The email text itself lives in the `compose` i18n namespace under
// `feedback.<type>.{subject,body}`; this module only decides which keys to use
// and formats the "technical info" line interpolated into the body.

/** Where every feedback email is addressed. */
export const FEEDBACK_RECIPIENT = 'hello@getemailops.com';

export type FeedbackType = 'general' | 'bug' | 'idea';

/** Ordered list of feedback kinds shown in the sidebar popover. */
export const FEEDBACK_TYPES: readonly FeedbackType[] = ['general', 'bug', 'idea'];

/** Raw runtime facts gathered from the OS / app version / AI config. */
export interface FeedbackTech {
  appVersion: string;
  /** Raw Tauri platform code, e.g. `macos`, `windows`, `linux`. */
  osPlatform: string;
  /** OS version string, e.g. `14.5.0`. May be empty if unavailable. */
  osVersion: string;
  /** CPU architecture, e.g. `aarch64`. May be empty if unavailable. */
  arch: string;
  /** True when the process is Rosetta-translated. Reported because `arch` is a
   *  compile-time constant: `x86_64` alone cannot tell a real Intel Mac (which
   *  cannot run the embedded AI runtime) from a translated one. */
  translated?: boolean;
  /** Active AI provider, e.g. `llamacpp`. */
  aiProvider: string;
  /** Active AI model, e.g. `qwen3.5-4b`. May be empty. */
  aiModel: string;
}

/** Values interpolated into the localized body's `— technical info —` line. */
export interface FeedbackTechInfo {
  version: string;
  os: string;
  aiProvider: string;
}

export interface FeedbackEmail {
  to: string;
  subject: string;
  body: string;
}

const OS_DISPLAY_NAMES: Record<string, string> = {
  macos: 'macOS',
  windows: 'Windows',
  linux: 'Linux',
  ios: 'iOS',
  android: 'Android',
};

/** Turn a raw Tauri platform code into a human-readable OS name. */
export function friendlyOsName(platform: string): string {
  const known = OS_DISPLAY_NAMES[platform];
  if (known) return known;
  if (!platform) return 'Unknown';
  return platform.charAt(0).toUpperCase() + platform.slice(1);
}

/** Collapse raw runtime facts into the three strings the body interpolates. */
export function formatFeedbackTech(tech: FeedbackTech): FeedbackTechInfo {
  const name = friendlyOsName(tech.osPlatform);
  const withVersion = tech.osVersion ? `${name} ${tech.osVersion}` : name;
  const archLabel = tech.translated ? `${tech.arch}, Rosetta` : tech.arch;
  const os = tech.arch ? `${withVersion} (${archLabel})` : withVersion;
  const aiProvider = tech.aiModel ? `${tech.aiProvider} / ${tech.aiModel}` : tech.aiProvider;
  return { version: tech.appVersion, os, aiProvider };
}

/** The exact `compose` namespace keys this module resolves. Typing `Translate`
 *  against this literal union (rather than plain `string`) keeps it compatible
 *  with i18next's key-typed `t` — a `string`-keyed function would not assign. */
type FeedbackKey = `compose:feedback.${FeedbackType}.subject` | `compose:feedback.${FeedbackType}.body`;

/** Minimal shape of the i18next translator this module needs. */
type Translate = (key: FeedbackKey, options?: Record<string, string>) => string;

/**
 * Build the recipient / subject / body for a feedback email in the user's
 * current language. `t` must resolve the `compose` namespace (pass the bound
 * i18next translator; fully-qualified `compose:…` keys work from any binding).
 */
export function buildFeedbackEmail(type: FeedbackType, t: Translate, tech: FeedbackTech): FeedbackEmail {
  const info = formatFeedbackTech(tech);
  return {
    to: FEEDBACK_RECIPIENT,
    subject: t(`compose:feedback.${type}.subject`),
    body: t(`compose:feedback.${type}.body`, { ...info }),
  };
}
