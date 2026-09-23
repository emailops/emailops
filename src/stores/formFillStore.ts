import { create } from 'zustand';

import type { FillableFormId } from '@/lib/chatToolEffects';

/**
 * The hand-off between a chat `fillForm` effect and the component that owns
 * the form.
 *
 * The chat has no business knowing which React tree renders a form, and a form
 * has no business listening to Tauri events. So the effect dispatcher drops the
 * filled values here, the owning component picks them up by form id, and both
 * sides stay ignorant of each other.
 *
 * This is the reusable half of "fill a form from chat": a second form needs a
 * `FormDef` on the Rust side, an id in `FILLABLE_FORM_IDS`, and a component
 * that calls `useChatFilledForm('<its id>')`. Nothing here changes.
 *
 * Deliberately NOT persisted: a pending fill is a hand-off that lasts until the
 * form mounts and reads it, not user state.
 */
export interface PendingFormFill {
  formId: FillableFormId;
  /** Only keys the form declares, already coerced by the backend parser. */
  values: Record<string, unknown>;
  /** Required fields the model could not fill, for the UI to highlight. */
  missingRequired: string[];
  /** Bumped on every fill so a second fill of the SAME form still re-applies,
   *  even when the values happen to be identical. Without it, "añade una
   *  columna" twice in a row would silently no-op the second time. */
  nonce: number;
}

interface FormFillStore {
  pending: PendingFormFill | null;
  /** Called by the chat effect dispatcher. */
  setFilledForm: (formId: FillableFormId, values: Record<string, unknown>, missingRequired: string[]) => void;
  /** Called by the owning component once it has applied the values. */
  clearFilledForm: () => void;
}

export const useFormFillStore = create<FormFillStore>((set, get) => ({
  pending: null,
  setFilledForm: (formId, values, missingRequired) =>
    set({ pending: { formId, values, missingRequired, nonce: (get().pending?.nonce ?? 0) + 1 } }),
  clearFilledForm: () => set({ pending: null }),
}));

/** Selector: the pending fill for `formId`, or `null`. */
export function selectPendingFill(state: FormFillStore, formId: FillableFormId): PendingFormFill | null {
  return state.pending?.formId === formId ? state.pending : null;
}

/** Subscribe a form component to the fills addressed to it. */
export function useChatFilledForm(formId: FillableFormId): PendingFormFill | null {
  return useFormFillStore((s) => selectPendingFill(s, formId));
}
