import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type Language, NATIVE_NAMES, SUPPORTED_LANGUAGES, useUiLanguage } from '@/i18n';
import { useAiStore } from '@/stores/aiStore';
import type { InboxLayout } from '@/types';
import { StepAddAccount } from './StepAddAccount';
import { StepAiBackend } from './StepAiBackend';
import { StepAiChoice } from './StepAiChoice';
import { StepLayout } from './StepLayout';

/**
 * First-run onboarding wizard.
 *
 * Triggered from App.tsx when the `onboarding_completed` preference is
 * missing. The wizard owns its own step state and is dismissible only via
 * Skip on the final step or by completing the account add. Closing the app
 * mid-wizard does not persist completion — the wizard re-appears on next
 * launch so brand-new users aren't stranded.
 *
 * Steps:
 *   1: AI choice (auto-recommended based on hardware)
 *   2: AI backend & model download (only when AI is enabled)
 *   3: View layout (split / full-width)
 *   4: Add first email account (reuses existing add-account dialogs)
 *
 * Step 2 is skipped automatically when the user picks "plain" in step 1.
 * The displayed step indicator hides step 2 in that case so the user sees
 * a clean "X of 3" rather than "1, 3, 4".
 */
interface OnboardingWizardProps {
  currentLayout: InboxLayout;
  onChangeLayout: (layout: InboxLayout) => void;
  onComplete: () => Promise<void> | void;
}

type Step = 1 | 2 | 3 | 4;

function visibleSteps(aiEnabled: boolean): Step[] {
  return aiEnabled ? [1, 2, 3, 4] : [1, 3, 4];
}

export function OnboardingWizard({ currentLayout, onChangeLayout, onComplete }: OnboardingWizardProps) {
  const [step, setStep] = useState<Step>(1);
  // Reactive subscription — drives the Header/StepIndicator's "X of N" text.
  const { enabled: aiEnabled } = useAiStore();

  // `StepAiChoice.handleContinue` awaits `setEnabled(false)` and *then* calls
  // `onNext()`. Reading `aiEnabled` from the closure here would observe the
  // pre-await value (`true`), advancing the wizard to step 2 (AI backend) even
  // when the user explicitly opted out. `useAiStore.getState()` bypasses the
  // stale-closure problem by reading the live store value at click time.
  const goNext = (from: Step) => {
    const list = visibleSteps(useAiStore.getState().enabled);
    const idx = list.indexOf(from);
    if (idx >= 0 && idx < list.length - 1) setStep(list[idx + 1]);
  };
  const goBack = (from: Step) => {
    const list = visibleSteps(useAiStore.getState().enabled);
    const idx = list.indexOf(from);
    if (idx > 0) setStep(list[idx - 1]);
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-gray-900/95 p-6">
      <div className="bg-[#1f1f20] border border-gray-700 rounded-xl shadow-2xl w-full max-w-2xl flex flex-col max-h-[90vh] overflow-hidden">
        <Header step={step} aiEnabled={aiEnabled} />
        <div className="flex-1 overflow-y-auto px-8 py-6">
          {step === 1 && <StepAiChoice onNext={() => goNext(1)} />}
          {step === 2 && <StepAiBackend onBack={() => goBack(2)} onNext={() => goNext(2)} />}
          {step === 3 && (
            <StepLayout
              currentLayout={currentLayout}
              onChangeLayout={onChangeLayout}
              onBack={() => goBack(3)}
              onNext={() => goNext(3)}
            />
          )}
          {step === 4 && <StepAddAccount onBack={() => goBack(4)} onComplete={onComplete} />}
        </div>
      </div>
    </div>
  );
}

function Header({ step, aiEnabled }: { step: Step; aiEnabled: boolean }) {
  const { t } = useTranslation(['auth']);
  const titles: Record<Step, string> = {
    1: t('auth:onboarding.wizard.title1'),
    2: t('auth:onboarding.wizard.title2'),
    3: t('auth:onboarding.wizard.title3'),
    4: t('auth:onboarding.wizard.title4'),
  };
  const list = visibleSteps(aiEnabled);
  const displayIdx = Math.max(0, list.indexOf(step)) + 1;
  return (
    <div className="px-8 py-5 border-b border-gray-700 flex items-center justify-between">
      <div>
        <div className="text-xs uppercase tracking-wider text-gray-500">
          {t('auth:onboarding.wizard.stepOf', { current: displayIdx, total: list.length })}
        </div>
        <h1 className="text-xl font-semibold text-gray-100 mt-0.5">{titles[step]}</h1>
      </div>
      <div className="flex items-center gap-4">
        <LanguagePicker />
        <StepIndicator step={step} aiEnabled={aiEnabled} />
      </div>
    </div>
  );
}

/**
 * First-run language picker. The UI language is auto-detected from the OS
 * locale on boot; this lets a new user override it before going any further.
 * Persists to the `ui_language` SQLite preference via `useUiLanguage`.
 */
function LanguagePicker() {
  const { t } = useTranslation(['common']);
  const { language, setLanguage } = useUiLanguage();
  return (
    <select
      aria-label={t('common:language.selectLabel')}
      value={language}
      onChange={(e) => {
        void setLanguage(e.target.value as Language);
      }}
      className="bg-[#333] text-gray-300 border border-gray-600 rounded px-2 py-1 text-xs focus:border-primary-500 outline-none"
    >
      {SUPPORTED_LANGUAGES.map((code) => (
        <option key={code} value={code}>
          {NATIVE_NAMES[code]}
        </option>
      ))}
    </select>
  );
}

function StepIndicator({ step, aiEnabled }: { step: Step; aiEnabled: boolean }) {
  const list = visibleSteps(aiEnabled);
  const currentIdx = list.indexOf(step);
  return (
    <div className="flex items-center gap-1.5">
      {list.map((n, idx) => (
        <div key={n} className={`h-1.5 w-6 rounded-full ${idx <= currentIdx ? 'bg-primary-500' : 'bg-gray-700'}`} />
      ))}
    </div>
  );
}
