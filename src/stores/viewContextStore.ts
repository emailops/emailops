import { useMemo } from 'react';
import { create } from 'zustand';

import type { ChatViewContext } from '@/lib/api';

/**
 * What the user has on screen, for the chat to use as per-turn context.
 *
 * Two registrations, because two different parts of the app know two different
 * halves and neither should have to know the other:
 *
 *   - `base` — the main view or Settings tab, registered by `App`.
 *   - `form` — a fillable form currently open, registered by the component
 *     that renders it, together with its current values.
 *
 * A form wins, because it is the more specific thing in front of the user:
 * with the Create Lens dialog up, "añade una columna" is about the dialog, not
 * about the Lenses view behind it.
 *
 * Everything here is ephemeral and never persisted — it describes this moment,
 * and the backend re-validates the token before it can reach a prompt.
 */
export interface OpenFormContext {
  /** `form/<form id>`, matching a `FormDef` in the Rust registry. */
  token: string;
  /** The form's current values, keyed by the `FormDef` field keys. */
  values: Record<string, unknown>;
}

interface ViewContextStore {
  base: string | null;
  form: OpenFormContext | null;
  /** `view/<mode>` or `settings/<tab>`; `null` when nothing specific is up. */
  setBaseView: (token: string | null) => void;
  /** Called by a form component while it is open, on every change. */
  setOpenForm: (form: OpenFormContext | null) => void;
}

export const useViewContextStore = create<ViewContextStore>((set) => ({
  base: null,
  form: null,
  setBaseView: (token) => set({ base: token }),
  setOpenForm: (form) => set({ form }),
}));

/**
 * The context to send with the next turn, or `null` when there is nothing
 * worth telling the model. Pure, so the precedence rule is unit-tested rather
 * than inferred from a running app.
 */
export function selectChatViewContext(state: Pick<ViewContextStore, 'base' | 'form'>): ChatViewContext | null {
  if (state.form) {
    return { token: state.form.token, formValues: state.form.values };
  }
  if (state.base) {
    return { token: state.base, formValues: null };
  }
  return null;
}

/**
 * Subscribe the chat input to whatever is on screen right now.
 *
 * Selects the two STABLE slices and builds the context in a `useMemo`, rather
 * than passing `selectChatViewContext` to the store directly. The selector
 * returns a fresh object every call, and Zustand compares snapshots by
 * identity — handing it to `useViewContextStore` made React see a new snapshot
 * on every render and throw "The result of getSnapshot should be cached to
 * avoid an infinite loop", taking `<ChatPanel>` down with it.
 */
export function useChatViewContext(): ChatViewContext | null {
  const base = useViewContextStore((s) => s.base);
  const form = useViewContextStore((s) => s.form);
  return useMemo(() => selectChatViewContext({ base, form }), [base, form]);
}
