// The OpenAI-compatible server in Settings → AI: which saves need the remote
// server disclosure, and where the email index is sent. Pure. The backend
// (`services::ai_server::plan_server_settings`) applies the same rules and
// refuses a save that skipped the disclosure.

import type { AiConfigState, EmbeddingSource } from './types';

/** Whether `host` (as `URL.hostname` gives it) is this machine. */
function isLoopbackHost(host: string): boolean {
  if (host === 'localhost' || host === '[::1]') return true;
  return /^127(\.\d{1,3}){3}$/.test(host);
}

/**
 * The host the user must be told about before `baseUrl` is saved: a remote
 * server's, unless it is `disclosedHost`, the one already confirmed. `null`
 * for a server on this machine, an already confirmed host, or a URL that
 * cannot be read (the backend reports what is wrong with it).
 */
export function disclosureHost(baseUrl: string, disclosedHost: string | null): string | null {
  let host: string;
  try {
    host = new URL(baseUrl.trim()).hostname.toLowerCase();
  } catch {
    return null;
  }
  if (host === '' || isLoopbackHost(host)) return null;
  return host === disclosedHost?.toLowerCase() ? null : host;
}

/** Whether the email index is computed by OpenRouter with this choice. */
export function sendsIndexToOpenRouter(
  provider: AiConfigState['provider'],
  embeddingSource: EmbeddingSource | undefined,
): boolean {
  return provider === 'openrouter' || (provider === 'openai_compatible' && embeddingSource === 'openrouter');
}

/**
 * Whether AI requests leave this machine with `provider`: OpenRouter, or an
 * OpenAI-compatible server whose saved `serverBaseUrl` is not on this machine.
 * Unset (the in-app default) and Ollama run here.
 */
export function aiRunsRemotely(provider: string | null, serverBaseUrl: string | null): boolean {
  if (provider === 'openrouter') return true;
  if (provider !== 'openai_compatible') return false;
  return serverBaseUrl != null && disclosureHost(serverBaseUrl, null) != null;
}
