// Custom dropdown replacing native <select> app-wide. WebKitGTK on Linux
// renders a native <select>'s option popup via the GTK theme, not page CSS,
// so a native select shows a light popup even inside this app's dark
// surfaces — owning the popup markup here lets it actually be styled.

import { useEffect, useRef, useState } from 'react';

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
}

const TRIGGER_SIZE_CLASSES: Record<'xs' | 'sm' | 'md', string> = {
  xs: 'px-2 py-1 text-xs',
  sm: 'px-3 py-2 text-sm',
  md: 'px-3 py-2.5 text-sm',
};

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
}: SelectProps<T>) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
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
        className={`flex items-center justify-between gap-1.5 bg-[#333] text-gray-300 border border-gray-600 rounded outline-none focus:border-primary-500 disabled:opacity-60 disabled:cursor-not-allowed ${TRIGGER_SIZE_CLASSES[size]} ${fullWidth ? 'w-full' : ''} ${className}`}
      >
        <span className={selected ? '' : 'text-gray-500'}>{selected ? selected.label : (placeholder ?? '')}</span>
        <svg className="w-3 h-3 text-gray-400 shrink-0" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
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
          className={`absolute ${align === 'right' ? 'right-0' : 'left-0'} mt-1 z-20 max-h-64 overflow-y-auto bg-[#333] border border-gray-600 rounded shadow-lg py-1 ${fullWidth ? 'w-full' : 'min-w-[8rem]'}`}
        >
          {options.map((opt) => (
            <button
              key={opt.value}
              type="button"
              role="option"
              aria-selected={opt.value === value}
              disabled={opt.disabled}
              onClick={() => selectOption(opt)}
              className={`block w-full text-left px-3 py-1.5 text-sm hover:bg-[#444] disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:bg-transparent ${
                opt.value === value ? 'text-primary-400' : 'text-gray-300'
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
