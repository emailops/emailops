// Validation helpers for the comma-separated scope inputs.
// Shared between LensCreateModal and LensScopeEditor so both prevent the
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

export interface ValidatedList {
  values: string[];
  error: string | null;
}

/** Parse + validate the Sender domains input. */
export function validateSenderDomains(raw: string): ValidatedList {
  const values = parseList(raw);
  for (const v of values) {
    if (v.includes('@')) {
      return {
        values,
        error: `"${v}" looks like an email — put it in "Sender emails" instead, or use just the domain (e.g. "${v.split('@')[1] ?? ''}").`,
      };
    }
    if (!DOMAIN_RE.test(v)) {
      return {
        values,
        error: `"${v}" is not a valid domain (expected something like "stripe.com").`,
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
        error: `"${v}" is not a valid email address.`,
      };
    }
  }
  return { values, error: null };
}
