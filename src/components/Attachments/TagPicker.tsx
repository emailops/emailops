import { useId, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

interface TagPickerProps {
  value: string[];
  onChange: (tags: string[]) => void;
  /** Tags already used by rules or collected attachments. */
  existingTags: string[];
  /** Id for the text input, so a `<label htmlFor>` can name it. */
  inputId?: string;
}

const sameTag = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'accent' }) === 0;

/**
 * Tag field for attachment rules: selected tags as removable chips, and a
 * filterable list of the tags already in use with an option to create a new
 * one — so the same tag is reused instead of retyped with a variant spelling.
 */
export function TagPicker({ value, onChange, existingTags, inputId }: TagPickerProps) {
  const { t } = useTranslation(['attachments']);
  const [query, setQuery] = useState('');
  const [open, setOpen] = useState(false);
  // Keyboard-highlighted option: an index into `options` (-1 = none).
  const [active, setActive] = useState(-1);
  const listId = useId();

  const trimmed = query.trim();
  const suggestions = useMemo(() => {
    const needle = trimmed.toLowerCase();
    return existingTags
      .filter((tag) => !value.some((v) => sameTag(v, tag)))
      .filter((tag) => tag.toLowerCase().includes(needle))
      .sort((a, b) => Number(!a.toLowerCase().startsWith(needle)) - Number(!b.toLowerCase().startsWith(needle)));
  }, [existingTags, value, trimmed]);
  // Tags are stored as JSON, but a comma reads as a separator everywhere the
  // tag is shown, so a new tag must not contain one.
  const isValidNew = (tag: string) => tag.length > 0 && !tag.includes(',');
  const canCreate = isValidNew(trimmed) && ![...existingTags, ...value].some((tag) => sameTag(tag, trimmed));
  // Everything the list offers, in order: existing tags, then "create".
  const options = canCreate ? [...suggestions, trimmed] : suggestions;
  const showList = open && options.length > 0;
  const optionId = (index: number) => `${listId}-option-${index}`;

  const add = (tag: string) => {
    if (!value.some((v) => sameTag(v, tag))) onChange([...value, tag]);
    setQuery('');
    setActive(-1);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      setOpen(true);
      if (options.length === 0) return;
      const step = e.key === 'ArrowDown' ? 1 : -1;
      setActive((prev) => (prev + step + options.length) % options.length);
    } else if (e.key === 'Escape') {
      if (showList) e.stopPropagation(); // close the list, not the dialog
      setOpen(false);
      setActive(-1);
    } else if (e.key === 'Enter') {
      if (showList && active >= 0 && active < options.length) {
        e.preventDefault();
        add(options[active]);
      } else if (trimmed) {
        e.preventDefault();
        const existing = suggestions.find((s) => sameTag(s, trimmed));
        if (existing) add(existing);
        else if (isValidNew(trimmed)) add(trimmed);
      }
    } else if (e.key === 'Backspace' && !query && value.length > 0) {
      onChange(value.slice(0, -1));
    }
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
          id={inputId}
          type="text"
          role="combobox"
          aria-expanded={showList}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={showList && active >= 0 ? optionId(active) : undefined}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setOpen(true);
            setActive(-1);
          }}
          onFocus={() => setOpen(true)}
          onBlur={() => {
            setOpen(false);
            setActive(-1);
          }}
          onKeyDown={onKeyDown}
          placeholder={value.length === 0 ? t('attachments:rules.tagsPlaceholder') : ''}
          className="flex-1 min-w-[8rem] px-1 py-0.5 text-sm outline-none bg-transparent"
        />
      </div>
      {showList && (
        <div
          id={listId}
          role="listbox"
          className="absolute z-10 mt-1 w-full max-h-48 overflow-y-auto bg-white border border-gray-200 rounded-lg shadow-lg py-1"
        >
          {suggestions.map((tag, index) => (
            <div
              key={tag}
              id={optionId(index)}
              role="option"
              aria-selected={active === index}
              tabIndex={-1}
              // mousedown, not click: it fires before the input's blur closes the list.
              onMouseDown={(e) => {
                e.preventDefault();
                add(tag);
              }}
              className={`px-3 py-1.5 text-sm text-gray-700 hover:bg-gray-100 cursor-pointer ${active === index ? 'bg-gray-100' : ''}`}
            >
              {tag}
            </div>
          ))}
          {canCreate && (
            <div
              id={optionId(suggestions.length)}
              role="option"
              aria-selected={active === suggestions.length}
              tabIndex={-1}
              onMouseDown={(e) => {
                e.preventDefault();
                add(trimmed);
              }}
              className={`px-3 py-1.5 text-sm text-primary-700 hover:bg-primary-50 cursor-pointer border-t border-gray-100 ${active === suggestions.length ? 'bg-primary-50' : ''}`}
            >
              {t('attachments:rules.createTag', { tag: trimmed })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
