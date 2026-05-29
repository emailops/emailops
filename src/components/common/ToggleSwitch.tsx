import type { ReactNode } from 'react';

/**
 * Dark-themed toggle switch with optional label/description.
 *
 * The switch on its own (no label/description) is also valid — pass only
 * `checked` and `onChange`. The label row layout matches the existing
 * Settings UI conventions (label left, switch right, description under label).
 */
interface ToggleSwitchProps {
  checked: boolean;
  onChange: (next: boolean) => void;
  label?: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
  /** Adds an aria-label when there's no visible label. */
  ariaLabel?: string;
}

export function ToggleSwitch({ checked, onChange, label, description, disabled, ariaLabel }: ToggleSwitchProps) {
  const button = (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      onClick={() => onChange(!checked)}
      disabled={disabled}
      className={`relative mt-0.5 inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-2 focus:ring-offset-[#252526] disabled:cursor-not-allowed disabled:opacity-50 ${
        checked ? 'bg-primary-600' : 'bg-gray-600'
      }`}
    >
      <span
        className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow ring-0 transition duration-200 ${
          checked ? 'translate-x-4' : 'translate-x-0'
        }`}
      />
    </button>
  );

  if (!label && !description) return button;

  return (
    <div className="flex items-start justify-between gap-4">
      <div className="min-w-0 flex-1">
        {label && <div className="text-sm font-medium text-gray-200">{label}</div>}
        {description && <div className="mt-0.5 text-xs text-gray-500">{description}</div>}
      </div>
      {button}
    </div>
  );
}
