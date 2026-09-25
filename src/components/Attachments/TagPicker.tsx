import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

interface TagPickerProps {
  value: string[];
  onChange: (tags: string[]) => void;
  /** Tags already used by rules or collected attachments. */
  existingTags: string[];
}

const sameTag = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'accent' }) === 0;

/**
 * Tag field for attachment rules: selected tags as removable chips, and a
 * filterable list of the tags already in use with an option to create a new
 * one — so the same tag is reused instead of retyped with a variant spelling.
 */
export function TagPicker({ value, onChange, existingTags }: TagPickerProps) {
  const { t } = useTranslation(['attachments']);
  const [query, setQuery] = useState('');
  const [open, setOpen] = useState(false);

  const trimmed = query.trim();
  const suggestions = useMemo(() => {
    const needle = trimmed.toLowerCase();
    return existingTags
      .filter((tag) => !value.some((v) => sameTag(v, tag)))
      .filter((tag) => tag.toLowerCase().includes(needle))
      .sort((a, b) => Number(!a.toLowerCase().startsWith(needle)) - Number(!b.toLowerCase().startsWith(needle)));
  }, [existingTags, value, trimmed]);
  const canCreate =
    trimmed.length > 0 && ![...existingTags, ...value].some((tag) => sameTag(tag, trimmed)) && !trimmed.includes(',');

  const add = (tag: string) => {
    if (!value.some((v) => sameTag(v, tag))) onChange([...value, tag]);
    setQuery('');
  };

  return (
    <div className="relative">
      <div className="flex flex-wrap items-center gap-1 w-full px-2 py-1.5 border border-gray-300 rounded-lg focus-within:ring-2 focus-within:ring-primary-500 focus-within:border-transparent bg-white">
        {value.map((tag) => (
          <span
            key={tag}
            className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-xs font-medium bg-primary-100 text-primary-700"
          >
            {tag}
            <button
              type="button"
              aria-label={t('attachments:rules.removeTag', { tag })}
              onClick={() => onChange(value.filter((v) => v !== tag))}
              className="text-primary-500 hover:text-primary-800"
            >
              ×
            </button>
          </span>
        ))}
        <input
          type="text"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onBlur={() => setOpen(false)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && trimmed) {
              e.preventDefault();
              add(suggestions.find((s) => sameTag(s, trimmed)) ?? trimmed);
            } else if (e.key === 'Backspace' && !query && value.length > 0) {
              onChange(value.slice(0, -1));
            }
          }}
          placeholder={value.length === 0 ? t('attachments:rules.tagsPlaceholder') : ''}
          className="flex-1 min-w-[8rem] px-1 py-0.5 text-sm outline-none bg-transparent"
        />
      </div>
      {open && (suggestions.length > 0 || canCreate) && (
        <div
          role="listbox"
          className="absolute z-10 mt-1 w-full max-h-48 overflow-y-auto bg-white border border-gray-200 rounded-lg shadow-lg py-1"
        >
          {suggestions.map((tag) => (
            <div
              key={tag}
              role="option"
              aria-selected={false}
              tabIndex={-1}
              // mousedown, not click: it fires before the input's blur closes the list.
              onMouseDown={(e) => {
                e.preventDefault();
                add(tag);
              }}
              className="px-3 py-1.5 text-sm text-gray-700 hover:bg-gray-100 cursor-pointer"
            >
              {tag}
            </div>
          ))}
          {canCreate && (
            <div
              role="option"
              aria-selected={false}
              tabIndex={-1}
              onMouseDown={(e) => {
                e.preventDefault();
                add(trimmed);
              }}
              className="px-3 py-1.5 text-sm text-primary-700 hover:bg-primary-50 cursor-pointer border-t border-gray-100"
            >
              {t('attachments:rules.createTag', { tag: trimmed })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
