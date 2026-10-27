/**
 * Moving the data folder: pick a folder (or the default) → confirm → copy
 * with progress → the app restarts. A failed copy changes nothing, so it goes
 * back to the confirmation with the error, ready to retry or cancel.
 */

export interface DataMoveProgressState {
  copiedBytes: number;
  totalBytes: number;
}

interface Destination {
  toDefault: boolean;
  target: string;
  neededBytes: number;
}

export type DataMoveState =
  | { phase: 'idle' }
  | ({ phase: 'confirm'; error: string | null } & Destination)
  | ({ phase: 'moving'; error: string | null; progress: DataMoveProgressState | null } & Destination);

export type DataMoveAction =
  | ({ type: 'picked' } & Destination)
  | { type: 'cancelled' }
  | { type: 'confirmed' }
  | ({ type: 'progress' } & DataMoveProgressState)
  | { type: 'failed'; error: string };

export const IDLE: DataMoveState = { phase: 'idle' };

export function dataMoveReducer(state: DataMoveState, action: DataMoveAction): DataMoveState {
  switch (action.type) {
    case 'picked':
      return {
        phase: 'confirm',
        toDefault: action.toDefault,
        target: action.target,
        neededBytes: action.neededBytes,
        error: null,
      };
    case 'cancelled':
      // The copy cannot be interrupted halfway; the dialog stays until it ends.
      return state.phase === 'moving' ? state : IDLE;
    case 'confirmed':
      return state.phase === 'confirm' ? { ...state, phase: 'moving', progress: null } : state;
    case 'progress':
      return state.phase === 'moving'
        ? { ...state, progress: { copiedBytes: action.copiedBytes, totalBytes: action.totalBytes } }
        : state;
    case 'failed': {
      if (state.phase !== 'moving') return state;
      const { progress: _progress, ...rest } = state;
      return { ...rest, phase: 'confirm', error: action.error };
    }
  }
}
