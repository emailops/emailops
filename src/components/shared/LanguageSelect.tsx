// Thin domain wrapper around the generic Select — see Select.tsx for why a
// custom dropdown exists at all (native <select> popups render light/native
// on Linux regardless of page CSS).

import { type Language, NATIVE_NAMES, SUPPORTED_LANGUAGES } from '@/i18n';
import { Select } from './Select';

const LANGUAGE_OPTIONS = SUPPORTED_LANGUAGES.map((code) => ({ value: code, label: NATIVE_NAMES[code] }));

interface LanguageSelectProps {
  value: Language;
  onChange: (language: Language) => void;
  ariaLabel: string;
  disabled?: boolean;
  size?: 'xs' | 'sm';
}

export function LanguageSelect({ value, onChange, ariaLabel, disabled = false, size = 'sm' }: LanguageSelectProps) {
  return (
    <Select
      value={value}
      options={LANGUAGE_OPTIONS}
      onChange={onChange}
      ariaLabel={ariaLabel}
      disabled={disabled}
      size={size}
      align="right"
    />
  );
}
