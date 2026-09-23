import type { LensColumnType, LensDirection } from '@/types';

import type { DraftColumn } from './lensDraft';

/**
 * Backend fill values → the Create Lens form's draft state.
 *
 * The backend hands over only keys the form declared, already coerced to their
 * declared kinds (`services::forms::filler`), so this is a shape translation,
 * not validation: flat `scope*` keys become the individual pieces of form
 * state, and the column list becomes `DraftColumn`s with the modal's
 * comma-separated `enumValues` convention.
 *
 * Pure and total — every field is optional, and anything unrecognised is left
 * out rather than defaulted, so a partial fill leaves the rest of the form on
 * whatever the user already had.
 */
export interface LensFormPrefill {
  name?: string;
  icon?: string;
  mailboxes?: string[];
  categories?: string[];
  direction?: LensDirection;
  senderDomains?: string;
  query?: string;
  prompt?: string;
  columns?: DraftColumn[];
}

const COLUMN_TYPES: readonly LensColumnType[] = [
  'string',
  'text',
  'number',
  'currency',
  'date',
  'boolean',
  'enum',
  'email',
  'url',
];

const DIRECTIONS: readonly LensDirection[] = ['inbound', 'outbound', 'either'];

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim().length > 0 ? v.trim() : undefined;
}

function strList(v: unknown): string[] | undefined {
  if (!Array.isArray(v)) return undefined;
  const items = v.filter((x): x is string => typeof x === 'string' && x.trim().length > 0).map((x) => x.trim());
  return items.length > 0 ? items : undefined;
}

function bool(v: unknown): boolean {
  return v === true;
}

function toDraftColumn(raw: unknown): DraftColumn | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null;
  const row = raw as Record<string, unknown>;
  const key = str(row.key);
  const label = str(row.label);
  // The backend already discards a column missing its required sub-fields, but
  // a version skew must not put a keyless row into the form.
  if (!key || !label) return null;
  const rawType = str(row.type);
  const type = COLUMN_TYPES.find((t) => t === rawType) ?? 'string';
  return {
    key,
    label,
    type,
    description: str(row.description) ?? '',
    required: bool(row.required),
    isUniqueKey: bool(row.isUniqueKey),
    enumValues: (strList(row.enumValues) ?? []).join(', '),
  };
}

export function toLensFormPrefill(values: Record<string, unknown>): LensFormPrefill {
  const prefill: LensFormPrefill = {};

  const name = str(values.name);
  if (name) prefill.name = name;

  const icon = str(values.icon);
  if (icon) prefill.icon = icon;

  const mailboxes = strList(values.scopeMailboxes);
  if (mailboxes) prefill.mailboxes = mailboxes;

  const categories = strList(values.scopeCategories);
  if (categories) prefill.categories = categories;

  const rawDirection = str(values.scopeDirection);
  const direction = DIRECTIONS.find((d) => d === rawDirection);
  if (direction) prefill.direction = direction;

  const domains = strList(values.scopeSenderDomains);
  if (domains) prefill.senderDomains = domains.join(', ');

  const query = str(values.scopeQuery);
  if (query) prefill.query = query;

  const prompt = str(values.promptText);
  if (prompt) prefill.prompt = prompt;

  if (Array.isArray(values.columns)) {
    const columns = values.columns.map(toDraftColumn).filter((c): c is DraftColumn => c !== null);
    if (columns.length > 0) prefill.columns = columns;
  }

  return prefill;
}

/**
 * The Create Lens form's draft state → the backend `FormDef` field keys.
 *
 * The inverse of {@link toLensFormPrefill}, and the reason "añade una columna
 * de IVA" can build on what is already on screen: the chat sends these values
 * back as the form's current state, the model returns the whole object with
 * its change applied, and the mapper above puts it back in the form.
 *
 * Empty and default-ish values are omitted rather than sent as blanks, so the
 * model is never told the user "chose" an empty name.
 */
export function toLensFormValues(state: LensFormPrefill): Record<string, unknown> {
  const values: Record<string, unknown> = {};
  if (state.name) values.name = state.name;
  if (state.icon) values.icon = state.icon;
  if (state.mailboxes?.length) values.scopeMailboxes = state.mailboxes;
  if (state.categories?.length) values.scopeCategories = state.categories;
  if (state.direction) values.scopeDirection = state.direction;
  const domains = (state.senderDomains ?? '')
    .split(',')
    .map((d) => d.trim())
    .filter(Boolean);
  if (domains.length) values.scopeSenderDomains = domains;
  if (state.query) values.scopeQuery = state.query;
  if (state.prompt) values.promptText = state.prompt;
  if (state.columns?.length) {
    values.columns = state.columns
      .filter((c) => c.key.trim().length > 0)
      .map((c) => {
        const row: Record<string, unknown> = {
          key: c.key.trim(),
          label: c.label.trim(),
          type: c.type,
        };
        if (c.description.trim()) row.description = c.description.trim();
        if (c.required) row.required = true;
        if (c.isUniqueKey) row.isUniqueKey = true;
        const enumValues = c.enumValues
          .split(',')
          .map((v) => v.trim())
          .filter(Boolean);
        if (enumValues.length) row.enumValues = enumValues;
        return row;
      });
  }
  return values;
}
