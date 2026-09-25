import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { describe, expect, it, vi } from 'vitest';
import { LensesSettings } from './LensesSettings';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

describe('LensesSettings', () => {
  it('no longer labels Lenses as experimental', () => {
    const container = document.createElement('div');
    const root = createRoot(container);
    act(() => root.render(<LensesSettings enabled={true} onChangeEnabled={() => {}} />));
    expect(container.textContent).toContain('settings:lenses.title');
    expect(container.textContent).not.toContain('settings:dialog.experimental');
    act(() => root.unmount());
  });
});
