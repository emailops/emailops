import { useTranslation } from 'react-i18next';
import type { InboxLayout } from '@/types';

export function StepLayout({
  currentLayout,
  onChangeLayout,
  onBack,
  onNext,
}: {
  currentLayout: InboxLayout;
  onChangeLayout: (layout: InboxLayout) => void;
  onBack: () => void;
  onNext: () => void;
}) {
  const { t } = useTranslation(['auth']);
  return (
    <div className="space-y-5">
      <p className="text-sm text-gray-400">{t('auth:onboarding.layout.intro')}</p>

      <div className="grid grid-cols-2 gap-3">
        <LayoutCard
          selected={currentLayout === 'split'}
          onSelect={() => onChangeLayout('split')}
          title={t('auth:onboarding.layout.splitTitle')}
          description={t('auth:onboarding.layout.splitDesc')}
          icon={<SplitIcon />}
        />
        <LayoutCard
          selected={currentLayout === 'full-width'}
          onSelect={() => onChangeLayout('full-width')}
          title={t('auth:onboarding.layout.fullWidthTitle')}
          description={t('auth:onboarding.layout.fullWidthDesc')}
          icon={<FullWidthIcon />}
        />
      </div>

      <div className="flex justify-between">
        <button
          onClick={onBack}
          className="px-4 py-2 text-sm text-gray-400 hover:text-gray-200 hover:bg-gray-800 rounded transition-colors"
        >
          {t('auth:onboarding.layout.back')}
        </button>
        <button onClick={onNext} className="px-5 py-2 bg-primary-600 text-white rounded text-sm hover:bg-primary-500">
          {t('auth:onboarding.layout.continue')}
        </button>
      </div>
    </div>
  );
}

function LayoutCard({
  selected,
  onSelect,
  title,
  description,
  icon,
}: {
  selected: boolean;
  onSelect: () => void;
  title: string;
  description: string;
  icon: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={`text-left p-4 rounded-lg border-2 transition-colors ${
        selected ? 'border-primary-500 bg-primary-900/15' : 'border-gray-700 bg-[#27272a] hover:border-gray-500'
      }`}
    >
      <div className={`w-full ${selected ? 'text-primary-300' : 'text-gray-500'}`}>{icon}</div>
      <div className="text-sm font-semibold text-gray-100 mt-3">{title}</div>
      <div className="text-xs text-gray-500 mt-1">{description}</div>
    </button>
  );
}

function SplitIcon() {
  return (
    <svg viewBox="0 0 80 50" className="w-full h-12" fill="none">
      <rect x="1" y="1" width="78" height="48" rx="3" stroke="currentColor" strokeWidth="1.5" />
      <line x1="32" y1="1" x2="32" y2="49" stroke="currentColor" strokeWidth="1.5" />
      <rect x="5" y="7" width="22" height="3" rx="1" fill="currentColor" opacity="0.5" />
      <rect x="5" y="14" width="22" height="3" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="5" y="21" width="22" height="3" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="36" y="7" width="38" height="3" rx="1" fill="currentColor" opacity="0.6" />
      <rect x="36" y="14" width="32" height="2" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="36" y="19" width="35" height="2" rx="1" fill="currentColor" opacity="0.3" />
    </svg>
  );
}

function FullWidthIcon() {
  return (
    <svg viewBox="0 0 80 50" className="w-full h-12" fill="none">
      <rect x="1" y="1" width="78" height="48" rx="3" stroke="currentColor" strokeWidth="1.5" />
      <rect x="5" y="7" width="70" height="5" rx="1" fill="currentColor" opacity="0.5" />
      <rect x="5" y="16" width="70" height="5" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="5" y="25" width="70" height="5" rx="1" fill="currentColor" opacity="0.3" />
      <rect x="5" y="34" width="70" height="5" rx="1" fill="currentColor" opacity="0.3" />
    </svg>
  );
}
