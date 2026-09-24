// Validation helpers for the comma-separated scope inputs.
// Shared between LensCreateModal and LensConfigModal so both prevent the
// "full email in Sender domains" foot-gun that silently makes scope match
// zero rows (DB stores `sender_domain` as the part after `@`, so an entry
// like "user@example.com" never matches an email whose sender_domain is
// "example.com").

// Lowercase letters/digits, optional internal hyphens, dot-separated, at
// least one dot — enough to reject emails (no `@` allowed), bare words
// ("gmail"), and obvious whitespace damage.
const DOMAIN_RE = /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$/;

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function parseList(raw: string): string[] {
  return raw
    .split(',')
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean);
}

/** Translation-ready error: render with `t(`lenses:scope.errors.${code}`, params)`. */
export interface ScopeInputError {
  code: 'domainLooksLikeEmail' | 'invalidDomain' | 'invalidEmail';
  params: { value: string; domain?: string };
}

export interface ValidatedList {
  values: string[];
  error: ScopeInputError | null;
}

/** Parse + validate the Sender domains input. */
export function validateSenderDomains(raw: string): ValidatedList {
  const values = parseList(raw);
  for (const v of values) {
    if (v.includes('@')) {
      return {
        values,
        error: { code: 'domainLooksLikeEmail', params: { value: v, domain: v.split('@')[1] ?? '' } },
      };
    }
    if (!DOMAIN_RE.test(v)) {
      return {
        values,
        error: { code: 'invalidDomain', params: { value: v } },
      };
    }
  }
  return { values, error: null };
}

/** Parse + validate the Sender emails input. */
export function validateSenderEmails(raw: string): ValidatedList {
  const values = parseList(raw);
  for (const v of values) {
    if (!EMAIL_RE.test(v)) {
      return {
        values,
        error: { code: 'invalidEmail', params: { value: v } },
      };
    }
  }
  return { values, error: null };
}
