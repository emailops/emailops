import type { MemoryFact } from '@/types';
import { FactRow } from './FactRow';

const SECTION_ORDER: Array<{ kind: string; title: string; hint: string }> = [
  { kind: 'user', title: 'Profile', hint: 'Facts about you' },
  { kind: 'contact', title: 'Contacts', hint: 'Facts keyed by email address' },
  { kind: 'domain', title: 'Domains', hint: 'Facts about organisations / domains' },
  { kind: 'project', title: 'Projects', hint: 'Project or thread-scoped facts' },
];

interface StatusGroupProps {
  title: string;
  hint: string;
  total: number;
  kindMap: Record<string, MemoryFact[]>;
  selectedFactId: string | null;
  onSelectFact: (factId: string) => void;
}

/**
 * One status bucket (Promoted / Candidate / Retired) rendered as a section
 * containing per-`subjectKind` sub-sections. Catch-all handles any unknown
 * kind we haven't enumerated explicitly.
 */
export function StatusGroup({ title, hint, total, kindMap, selectedFactId, onSelectFact }: StatusGroupProps) {
  return (
    <section className="space-y-4">
      <div className="flex items-baseline gap-2 pb-1 border-b border-gray-200 dark:border-gray-700">
        <h2 className="text-sm font-bold uppercase tracking-wider text-gray-900 dark:text-gray-100">{title}</h2>
        <span className="text-xs text-gray-500 dark:text-gray-400">({total})</span>
        <span className="text-xs text-gray-400 dark:text-gray-500">· {hint}</span>
      </div>
      {SECTION_ORDER.map(({ kind, title: kindTitle, hint: kindHint }) => {
        const list = kindMap[kind];
        if (!list || list.length === 0) return null;
        return (
          <FactSection
            key={kind}
            title={kindTitle}
            hint={kindHint}
            facts={list}
            selectedFactId={selectedFactId}
            onSelectFact={onSelectFact}
          />
        );
      })}
      {Object.entries(kindMap)
        .filter(([k]) => !SECTION_ORDER.some((s) => s.kind === k))
        .map(([k, list]) => (
          <FactSection
            key={k}
            title={k}
            hint="Other"
            facts={list}
            selectedFactId={selectedFactId}
            onSelectFact={onSelectFact}
          />
        ))}
    </section>
  );
}

interface FactSectionProps {
  title: string;
  hint: string;
  facts: MemoryFact[];
  selectedFactId: string | null;
  onSelectFact: (factId: string) => void;
}

function FactSection({ title, hint, facts, selectedFactId, onSelectFact }: FactSectionProps) {
  return (
    <section>
      <div className="flex items-baseline gap-2 mb-2">
        <h2 className="text-xs font-semibold uppercase tracking-wider text-gray-700 dark:text-gray-300">
          {title} ({facts.length})
        </h2>
        <span className="text-xs text-gray-400 dark:text-gray-500">{hint}</span>
      </div>
      <ul className="space-y-2">
        {facts.map((fact) => (
          <FactRow key={fact.id} fact={fact} selected={selectedFactId === fact.id} onSelect={onSelectFact} />
        ))}
      </ul>
    </section>
  );
}
