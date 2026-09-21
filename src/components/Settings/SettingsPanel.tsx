interface SettingsPanelProps {
  children: React.ReactNode;
  /**
   * Rendered above the scroll container, so it stays visible while the body
   * scrolls. This is where error banners belong — never at the bottom of the
   * scroll area, where the user has to scroll to discover the failure.
   */
  header?: React.ReactNode;
  /** Rendered below the scroll container, so it stays visible while the body scrolls. */
  footer?: React.ReactNode;
}

/**
 * Shared chrome for a Settings tab: a flex-column frame filling the dialog's
 * panel area, with the body in its own scroll container.
 *
 * Module-scoped by definition, which is the point: every panel used to inline
 * this markup, and the two that wrapped it in a local `Shell` component got a
 * new component identity on each render and remounted their whole subtree
 * (see "Component Identity & Remounts" in src/CLAUDE.md).
 */
export function SettingsPanel({ children, header, footer }: SettingsPanelProps) {
  return (
    <div className="flex flex-col flex-1 min-h-0">
      {header}
      <div className="overflow-y-auto flex-1 px-6 py-5 space-y-6">{children}</div>
      {footer}
    </div>
  );
}
