// Editor for a Lens's columns: key, label, type, required / unique-key flags,
// description and (for list columns) the allowed values. Shared by the create
// dialog and the Config dialog; the caller owns the rows.

import { useTranslation } from 'react-i18next';

import { Select } from '@/components/shared/Select';
import type { LensColumnType } from '@/types';

import type { DraftColumn } from './lensDraft';

const COLUMN_TYPES: LensColumnType[] = [
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

function newColumn(): DraftColumn {
  return { key: '', label: '', type: 'string', description: '', required: false, isUniqueKey: false, enumValues: '' };
}

interface LensColumnsEditorProps {
  columns: DraftColumn[];
  onChange: (columns: DraftColumn[]) => void;
}

export function LensColumnsEditor({ columns, onChange }: LensColumnsEditorProps) {
  const { t } = useTranslation(['lenses']);
  const update = (idx: number, patch: Partial<DraftColumn>) =>
    onChange(columns.map((c, i) => (i === idx ? { ...c, ...patch } : c)));

  return (
    <section className="space-y-2">
      <div className="flex items-center justify-between">
        <h3 className="text-[11px] uppercase tracking-wider text-gray-500">{t('lenses:columns.title')}</h3>
        <button
          type="button"
          onClick={() => onChange([...columns, newColumn()])}
          className="rounded border border-gray-600 px-2 py-0.5 text-[11px] text-gray-200 hover:bg-gray-700"
        >
          {t('lenses:columns.add')}
        </button>
      </div>
      <div className="space-y-2">
        {columns.map((c, idx) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: column rows have no stable id during creation; reorder/remove would still re-render correctly because inputs are uncontrolled
          <div key={idx} className="rounded border border-gray-700 bg-[#1e1e1e]/60 p-3">
            <div className="grid grid-cols-12 gap-2">
              <label className="col-span-3 block">
                <span className="mb-1 block text-[10px] uppercase text-gray-500">{t('lenses:columns.key')}</span>
                <input
                  type="text"
                  data-testid="lens-create-column-key"
                  value={c.key}
                  onChange={(e) => update(idx, { key: e.target.value })}
                  placeholder="amount" // i18n-ignore: example column key (technical identifier)
                  className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1 text-gray-100 focus:border-blue-500 focus:outline-none"
                />
              </label>
              <label className="col-span-3 block">
                <span className="mb-1 block text-[10px] uppercase text-gray-500">{t('lenses:columns.label')}</span>
                <input
                  type="text"
                  value={c.label}
                  onChange={(e) => update(idx, { label: e.target.value })}
                  placeholder={t('lenses:columns.builtin.amount')}
                  className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1 text-gray-100 focus:border-blue-500 focus:outline-none"
                />
              </label>
              <label className="col-span-3 block">
                <span className="mb-1 block text-[10px] uppercase text-gray-500">{t('lenses:columns.type')}</span>
                <Select
                  value={c.type}
                  options={COLUMN_TYPES.map((colType) => ({
                    value: colType,
                    label: t(`lenses:columns.types.${colType}`),
                  }))}
                  onChange={(value) => update(idx, { type: value as LensColumnType })}
                  ariaLabel={t('lenses:columns.type')}
                  size="xs"
                  fullWidth
                />
              </label>
              <div className="col-span-2 flex items-end gap-3">
                <label className="flex items-center gap-1 text-[11px] text-gray-300">
                  <input
                    type="checkbox"
                    checked={c.required}
                    onChange={(e) => update(idx, { required: e.target.checked })}
                  />
                  {t('lenses:columns.required')}
                </label>
                <label
                  className="flex items-center gap-1 text-[11px] text-gray-300"
                  title={t('lenses:create.uniqueKeyTooltip')}
                >
                  <input
                    type="checkbox"
                    checked={c.isUniqueKey}
                    onChange={(e) => {
                      // Only one column can be unique key at a time.
                      if (e.target.checked) {
                        onChange(columns.map((col, i) => ({ ...col, isUniqueKey: i === idx })));
                      } else {
                        update(idx, { isUniqueKey: false });
                      }
                    }}
                  />
                  {t('lenses:create.uniqueKey')}
                </label>
              </div>
              <div className="col-span-1 flex items-end justify-end">
                <button
                  type="button"
                  onClick={() => onChange(columns.filter((_, i) => i !== idx))}
                  className="rounded p-1 text-gray-500 hover:bg-gray-700 hover:text-red-300"
                  title={t('lenses:create.removeColumn')}
                  aria-label={t('lenses:create.removeColumn')}
                >
                  ✕
                </button>
              </div>
            </div>
            <label className="mt-2 block">
              <span className="mb-1 block text-[10px] uppercase text-gray-500">{t('lenses:columns.description')}</span>
              <input
                type="text"
                value={c.description}
                onChange={(e) => update(idx, { description: e.target.value })}
                placeholder={t('lenses:create.descriptionPlaceholder')}
                className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1 text-gray-100 focus:border-blue-500 focus:outline-none"
              />
            </label>
            {c.type === 'enum' && (
              <label className="mt-2 block">
                <span className="mb-1 block text-[10px] uppercase text-gray-500">{t('lenses:columns.enumValues')}</span>
                <input
                  type="text"
                  value={c.enumValues}
                  onChange={(e) => update(idx, { enumValues: e.target.value })}
                  placeholder="paid, unpaid, refunded" // i18n-ignore: example enum values
                  className="w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1 text-gray-100 focus:border-blue-500 focus:outline-none"
                />
              </label>
            )}
          </div>
        ))}
      </div>
    </section>
  );
}
