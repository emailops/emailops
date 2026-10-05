import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { EoDocsAttachDialog } from '@/components/Documents/EoDocsAttachDialog';
import { useSharedDocsEnabledStore } from '@/stores/featureToggleStore';
import type { SharedDoc } from '@/types';

interface AttachMenuProps {
  /** The sending account: EO Docs are picked from its documents. */
  accountId: string;
  disabled?: boolean;
  /** Open the operating system's file picker. */
  onPickFiles: () => void;
  onPickEoDocs: (docs: SharedDoc[]) => void;
  className: string;
  /** Tooltip; defaults to "Attach files". */
  title?: string;
  /** The paperclip icon, as the composer draws it. */
  children: React.ReactNode;
}

/**
 * The composer's attach button. With EO Docs on, it asks where from — this
 * computer or EO Docs; with EO Docs off it opens the file picker directly,
 * as it always did.
 */
export function AttachMenu({
  accountId,
  disabled,
  onPickFiles,
  onPickEoDocs,
  className,
  title,
  children,
}: AttachMenuProps) {
  const { t } = useTranslation(['compose']);
  const { enabled: eoDocs } = useSharedDocsEnabledStore();
  const [open, setOpen] = useState(false);
  const [picking, setPicking] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener('mousedown', close);
    return () => window.removeEventListener('mousedown', close);
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        data-testid="compose-attach"
        disabled={disabled}
        onClick={() => (eoDocs ? setOpen((o) => !o) : onPickFiles())}
        className={className}
        title={title ?? t('compose:attachFiles')}
        aria-haspopup={eoDocs ? 'menu' : undefined}
        aria-expanded={eoDocs ? open : undefined}
      >
        {children}
      </button>
      {open && (
        <div
          role="menu"
          className="absolute bottom-full left-0 z-50 mb-1 w-48 overflow-hidden rounded-lg border border-gray-200 bg-white py-1 text-sm text-gray-700 shadow-lg"
        >
          <button
            type="button"
            role="menuitem"
            data-testid="compose-attach-computer"
            onClick={() => {
              setOpen(false);
              onPickFiles();
            }}
            className="w-full px-3 py-1.5 text-left hover:bg-gray-100"
          >
            {t('compose:attachFromComputer')}
          </button>
          <button
            type="button"
            role="menuitem"
            data-testid="compose-attach-eodocs"
            onClick={() => {
              setOpen(false);
              setPicking(true);
            }}
            className="w-full px-3 py-1.5 text-left hover:bg-gray-100"
          >
            {t('compose:attachFromEoDocs')}
          </button>
        </div>
      )}
      {picking && (
        <EoDocsAttachDialog
          accountId={accountId}
          onAttach={(docs) => {
            setPicking(false);
            onPickEoDocs(docs);
          }}
          onClose={() => setPicking(false)}
        />
      )}
    </div>
  );
}
