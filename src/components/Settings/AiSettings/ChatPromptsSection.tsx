import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { PromptEditorBlock } from '../PromptEditorBlock';

export function ChatPromptsSection() {
  const { t } = useTranslation(['common', 'settings']);
  const [showAdvanced, setShowAdvanced] = useState(false);
  return (
    <div>
      <div className="flex items-center justify-between mb-1">
        <label className="block text-sm font-medium text-gray-300">{t('settings:chatPrompts.title')}</label>
        <label className="flex items-center gap-2 text-xs text-gray-400 cursor-pointer">
          <input
            type="checkbox"
            checked={showAdvanced}
            onChange={(e) => setShowAdvanced(e.target.checked)}
            className="accent-primary-500"
          />
          {t('settings:chatPrompts.showAdvanced')}
        </label>
      </div>
      <p className="text-xs text-gray-500 mb-3">{t('settings:chatPrompts.description')}</p>
      <div className="space-y-4">
        <PromptEditorBlock promptId="chat.system" title={t('settings:chatPrompts.systemPrompt')} />
        {showAdvanced && (
          <>
            <PromptEditorBlock promptId="chat.query_rewrite" title={t('settings:chatPrompts.queryRewrite')} />
            <PromptEditorBlock promptId="chat.rerank" title={t('settings:chatPrompts.rerank')} />
          </>
        )}
      </div>
    </div>
  );
}
