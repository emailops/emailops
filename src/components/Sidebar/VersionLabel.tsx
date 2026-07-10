import { useEffect, useState } from 'react';
import { getBuildInfo } from '@/lib/api';
import { formatVersionLabel } from '@/lib/version';

/**
 * Version line under the EmailOps wordmark: "v0.6.2" for release builds,
 * "v0.6.2 (05ae613)" for anything built from an untagged commit. Renders
 * nothing until the build info resolves (or if the command fails) so the
 * header never shows a placeholder.
 */
export function VersionLabel() {
  const [label, setLabel] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getBuildInfo()
      .then((info) => {
        if (!cancelled) setLabel(formatVersionLabel(info));
      })
      .catch(() => {
        // Purely informational — a missing version line is better than an
        // error state in the sidebar header.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (!label) return null;
  return <p className="text-[11px] text-gray-500 mt-0.5">{label}</p>;
}
