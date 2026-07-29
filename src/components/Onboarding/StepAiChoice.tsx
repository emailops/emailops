import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { useAiStore } from '@/stores/aiStore';

export function StepAiChoice({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation(['auth']);
  const { setEnabled } = useAiStore();
  // Capability is RAM-based and platform-neutral. Keying this off
  // `appleSilicon`, as it used to, defaulted every Linux and Windows machine to
  // the no-AI client no matter how much memory it had.
  const [capability, setCapability] = useState<api.AiCapability | null>(null);
  const [choice, setChoice] = useState<'ai' | 'plain' | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    api
      .detectAiCapability()
      .then((cap) => {
        setCapability(cap);
        setChoice(cap.localAiCapable ? 'ai' : 'plain');
      })
      .catch(() => {
        // Probe failed: recommend the option that always works.
        setCapability(null);
        setChoice('plain');
      });
  }, []);

  const capable = capability?.localAiCapable ?? null;
  const ramCopy = {
    ram: String(capability?.totalRamGb ?? 0),
    minRam: String(capability?.minRamGbForLocalAi ?? 0),
  };

  const handleContinue = async () => {
    if (!choice) return;
    setSubmitting(true);
    try {
      await setEnabled(choice === 'ai');
      onNext();
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="space-y-5">
      <p className="text-sm text-gray-400">{t('auth:onboarding.aiChoice.intro')}</p>

      <div className="grid grid-cols-2 gap-3">
        <ChoiceCard
          selected={choice === 'ai'}
          onSelect={() => setChoice('ai')}
          title={t('auth:onboarding.aiChoice.useAi')}
          subtitle={
            capable === true
              ? t('auth:onboarding.aiChoice.useAiRecommended')
              : capable === false
                ? t('auth:onboarding.aiChoice.useAiAvailable', ramCopy)
                : t('auth:onboarding.aiChoice.detecting')
          }
        >
          <ul className="text-xs text-gray-400 space-y-1.5 mt-2">
            <li>{t('auth:onboarding.aiChoice.useAiBullet1')}</li>
            <li>{t('auth:onboarding.aiChoice.useAiBullet2')}</li>
            <li>{t('auth:onboarding.aiChoice.useAiBullet3')}</li>
            <li>{t('auth:onboarding.aiChoice.useAiBullet4')}</li>
          </ul>
          <p className="text-[11px] text-gray-500 mt-3">{t('auth:onboarding.aiChoice.useAiHardware', ramCopy)}</p>
        </ChoiceCard>

        <ChoiceCard
          selected={choice === 'plain'}
          onSelect={() => setChoice('plain')}
          title={t('auth:onboarding.aiChoice.plain')}
          subtitle={
            capable === false ? t('auth:onboarding.aiChoice.plainRecommended') : t('auth:onboarding.aiChoice.plainNoAi')
          }
        >
          <ul className="text-xs text-gray-400 space-y-1.5 mt-2">
            <li>{t('auth:onboarding.aiChoice.plainBullet1')}</li>
            <li>{t('auth:onboarding.aiChoice.plainBullet2')}</li>
            <li>{t('auth:onboarding.aiChoice.plainBullet3')}</li>
            <li>{t('auth:onboarding.aiChoice.plainBullet4')}</li>
            <li>{t('auth:onboarding.aiChoice.plainBullet5')}</li>
          </ul>
          <p className="text-[11px] text-gray-500 mt-3">{t('auth:onboarding.aiChoice.plainTurnOnLater')}</p>
        </ChoiceCard>
      </div>

      <p className="text-xs text-gray-500">{t('auth:onboarding.aiChoice.reversible')}</p>

      <div className="flex justify-end">
        <button
          onClick={() => void handleContinue()}
          disabled={!choice || submitting}
          className="px-5 py-2 bg-primary-600 text-white rounded text-sm hover:bg-primary-500 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {submitting ? t('auth:onboarding.aiChoice.saving') : t('auth:onboarding.aiChoice.continue')}
        </button>
      </div>
    </div>
  );
}

function ChoiceCard({
  selected,
  onSelect,
  title,
  subtitle,
  children,
}: {
  selected: boolean;
  onSelect: () => void;
  title: string;
  subtitle: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={`text-left p-4 rounded-lg border-2 transition-colors ${
        selected ? 'border-primary-500 bg-primary-900/15' : 'border-gray-700 bg-[#27272a] hover:border-gray-500'
      }`}
    >
      <div className="text-sm font-semibold text-gray-100">{title}</div>
      <div className={`text-xs mt-0.5 ${selected ? 'text-primary-300' : 'text-gray-500'}`}>{subtitle}</div>
      {children}
    </button>
  );
}
