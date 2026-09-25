import { beforeEach, describe, expect, it } from 'vitest';

import { selectPendingFill, useFormFillStore } from './formFillStore';

describe('formFillStore', () => {
  beforeEach(() => {
    useFormFillStore.getState().clearFilledForm();
  });

  it('addresses a fill to the form that should receive it', () => {
    useFormFillStore.getState().setFilledForm('lens.create', { name: 'Facturas' }, []);
    const state = useFormFillStore.getState();
    expect(selectPendingFill(state, 'lens.create')?.values).toMatchObject({ name: 'Facturas' });
  });

  it('carries the fields the model could not fill', () => {
    useFormFillStore.getState().setFilledForm('lens.create', {}, ['columns']);
    expect(selectPendingFill(useFormFillStore.getState(), 'lens.create')?.missingRequired).toEqual(['columns']);
  });

  it('reports nothing pending once the form has consumed it', () => {
    useFormFillStore.getState().setFilledForm('lens.create', { name: 'X' }, []);
    useFormFillStore.getState().clearFilledForm();
    expect(selectPendingFill(useFormFillStore.getState(), 'lens.create')).toBeNull();
  });

  // Regression: the nonce was derived from the CURRENT pending value
  // (`(get().pending?.nonce ?? 0) + 1`), but the form clears the store as soon
  // as it applies a fill — so the second fill of a conversation restarted at 1,
  // matched the nonce the form had already applied, and was silently dropped.
  // Symptom on 23/09/2026: "creame una lente…" filled 5 fields, "sí" said it
  // filled 8, and the form still showed the first 5.
  it('never reuses a nonce after the form consumed the previous fill', () => {
    useFormFillStore.getState().setFilledForm('lens.create', { name: 'first' }, []);
    const first = selectPendingFill(useFormFillStore.getState(), 'lens.create')?.nonce;

    // The form applies it and clears the hand-off, exactly as LensCreateModal does.
    useFormFillStore.getState().clearFilledForm();

    useFormFillStore.getState().setFilledForm('lens.create', { name: 'second' }, []);
    const second = selectPendingFill(useFormFillStore.getState(), 'lens.create')?.nonce;

    expect(second).toBeDefined();
    expect(second).not.toBe(first);
  });

  it('keeps nonces increasing across a whole conversation of fills', () => {
    const seen: number[] = [];
    for (let i = 0; i < 5; i += 1) {
      useFormFillStore.getState().setFilledForm('lens.create', { name: `fill-${i}` }, []);
      const n = selectPendingFill(useFormFillStore.getState(), 'lens.create')?.nonce;
      if (n !== undefined) seen.push(n);
      useFormFillStore.getState().clearFilledForm();
    }
    const sorted = [...seen].sort((a, b) => a - b);
    expect(seen).toEqual(sorted);
    expect(new Set(seen).size).toBe(seen.length);
  });

  it('does not hand a fill to a form it was not addressed to', () => {
    useFormFillStore.getState().setFilledForm('lens.create', { name: 'X' }, []);
    // A second fillable form would read its own id and must see nothing.
    expect(selectPendingFill(useFormFillStore.getState(), 'lens.create' as never)).not.toBeNull();
    expect(selectPendingFill({ pending: null }, 'lens.create')).toBeNull();
  });
});
