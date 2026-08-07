// Custom dropdown replacing native <select> on Linux only. WebKitGTK there
// renders a native <select>'s option popup via the GTK theme, not page CSS,
// so a native select shows a light popup even inside this app's dark
// surfaces — owning the popup markup lets it actually be styled. macOS and
// Windows don't have that bug, so they keep the real native <select> (OS
// popup + keyboard/a11y behavior for free) — see NEEDS_OWNED_POPUP below.

import { useEffect, useRef, useState } from 'react';
import { currentPlatform } from '@/lib/api';

export interface SelectOption<T extends string> {
  value: T;
  label: string;
  disabled?: boolean;
}

interface SelectProps<T extends string> {
  value: T;
  options: readonly SelectOption<T>[];
  onChange: (value: T) => void;
  ariaLabel: string;
  disabled?: boolean;
  size?: 'xs' | 'sm' | 'md';
  placeholder?: string;
  fullWidth?: boolean;
  className?: string;
  /** Which edge the popup panel hangs from. Default 'left'. */
  align?: 'left' | 'right';
  /** Surface this sits on: 'dark' (default) matches the app's dark chrome
   *  (Settings, Lenses, Calendar, LogPanel); 'light' matches the compose
   *  surfaces (white/gray-50 backgrounds). */
  variant?: 'dark' | 'light';
}

const TRIGGER_SIZE_CLASSES: Record<'xs' | 'sm' | 'md', string> = {
  xs: 'px-2 py-1 text-xs',
  sm: 'px-3 py-2 text-sm',
  md: 'px-3 py-2.5 text-sm',
};

const VARIANT_CLASSES = {
  dark: {
    trigger: 'bg-[#333] text-gray-300 border border-gray-600 focus:border-primary-500',
    placeholder: 'text-gray-500 dark:text-gray-400',
    popup: 'bg-[#333] border border-gray-600',
    optionHover: 'hover:bg-[#444]',
    optionText: 'text-gray-300',
    optionSelectedText: 'text-primary-400',
  },
  light: {
    trigger:
      'bg-white text-gray-900 border border-gray-300 focus:border-primary-500 focus:ring-2 focus:ring-primary-100 dark:bg-surface dark:text-gray-100 dark:border-gray-600',
    placeholder: 'text-gray-400 dark:text-gray-500',
    popup: 'bg-white border border-gray-200 dark:bg-surface dark:border-gray-700',
    optionHover: 'hover:bg-gray-50 dark:hover:bg-surface-raised',
    optionText: 'text-gray-700 dark:text-gray-300',
    optionSelectedText: 'text-primary-600 dark:text-primary-400',
  },
} as const;

// The host platform never changes mid-session, so this is computed once at
// module load rather than per render. Unknown platforms (tests, a plain
// browser) fall back to the owned-popup behavior — the same "assume
// non-Apple desktop" convention src/lib/platform.ts already uses.
const NEEDS_OWNED_POPUP = currentPlatform() !== 'macos' && currentPlatform() !== 'windows';

export function Select<T extends string>({
  value,
  options,
  onChange,
  ariaLabel,
  disabled = false,
  size = 'sm',
  placeholder,
  fullWidth = false,
  className = '',
  align = 'left',
  variant = 'dark',
}: SelectProps<T>) {
  const colors = VARIANT_CLASSES[variant];

  // Hooks below must run unconditionally on every render (NEEDS_OWNED_POPUP
  // is a module-level constant, never changes for a mounted instance, but
  // the lint rule can't know that) — the native-vs-owned branch happens only
  // in the returned JSX, after all hooks have been called.
  const [open, setOpen] = useState(false);
  const [openDirection, setOpenDirection] = useState<'down' | 'up'>('down');
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!NEEDS_OWNED_POPUP || !open) return;
    const onDocMouseDown = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onDocMouseDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onDocMouseDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  useEffect(() => {
    if (!NEEDS_OWNED_POPUP || !open || !containerRef.current) return;
    // A native <select> auto-flips its popup upward when there's no room
    // below (e.g. a trigger pinned to a bottom toolbar); this custom popup
    // needs the same or it renders clipped/off-screen there. One estimate
    // up front (not a measure-then-correct two-pass) is enough since the
    // trigger's own position doesn't change while the popup is open.
    const rect = containerRef.current.getBoundingClientRect();
    const estimatedPopupHeight = Math.min(options.length * 32 + 8, 256); // matches max-h-64
    const spaceBelow = window.innerHeight - rect.bottom;
    const spaceAbove = rect.top;
    setOpenDirection(spaceBelow < estimatedPopupHeight && spaceAbove > spaceBelow ? 'up' : 'down');
  }, [open, options.length]);

  if (!NEEDS_OWNED_POPUP) {
    return (
      <select
        aria-label={ariaLabel}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value as T)}
        className={`rounded outline-none disabled:opacity-60 disabled:cursor-not-allowed ${colors.trigger} ${TRIGGER_SIZE_CLASSES[size]} ${fullWidth ? 'w-full' : ''} ${className}`}
      >
        {placeholder !== undefined && (
          <option value="" disabled hidden>
            {placeholder}
          </option>
        )}
        {options.map((opt) => (
          <option key={opt.value} value={opt.value} disabled={opt.disabled}>
            {opt.label}
          </option>
        ))}
      </select>
    );
  }

  const selected = options.find((o) => o.value === value);

  const selectOption = (opt: SelectOption<T>) => {
    if (opt.disabled) return;
    onChange(opt.value);
    setOpen(false);
  };

  return (
    <div ref={containerRef} className={`relative inline-block ${fullWidth ? 'w-full' : ''}`}>
      <button
        type="button"
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => {
          if (!disabled) setOpen((v) => !v);
        }}
        className={`flex items-center justify-between gap-1.5 rounded outline-none disabled:opacity-60 disabled:cursor-not-allowed ${colors.trigger} ${TRIGGER_SIZE_CLASSES[size]} ${fullWidth ? 'w-full' : ''} ${className}`}
      >
        <span className={selected ? '' : colors.placeholder}>{selected ? selected.label : (placeholder ?? '')}</span>
        <svg
          className="w-3 h-3 text-gray-400 shrink-0 dark:text-gray-500"
          viewBox="0 0 20 20"
          fill="currentColor"
          aria-hidden="true"
        >
          <path
            fillRule="evenodd"
            d="M5.23 7.21a.75.75 0 011.06.02L10 11.06l3.71-3.83a.75.75 0 111.08 1.04l-4.24 4.38a.75.75 0 01-1.08 0L5.21 8.27a.75.75 0 01.02-1.06z"
            clipRule="evenodd"
          />
        </svg>
      </button>

      {open && (
        <div
          role="listbox"
          aria-label={ariaLabel}
          className={`absolute ${align === 'right' ? 'right-0' : 'left-0'} ${
            openDirection === 'up' ? 'bottom-full mb-1' : 'top-full mt-1'
          } z-20 max-h-64 overflow-y-auto rounded shadow-lg py-1 ${colors.popup} ${fullWidth ? 'w-full' : 'min-w-[8rem]'}`}
        >
          {options.map((opt) => (
            <button
              key={opt.value}
              type="button"
              role="option"
              aria-selected={opt.value === value}
              disabled={opt.disabled}
              onClick={() => selectOption(opt)}
              className={`block w-full text-left px-3 py-1.5 text-sm disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:bg-transparent ${colors.optionHover} ${
                opt.value === value ? colors.optionSelectedText : colors.optionText
              }`}
            >
              {opt.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
