import { describe, expect, it } from 'vitest';
import { type DataMoveState, dataMoveReducer, IDLE } from './dataMoveFlow';

const confirming: DataMoveState = {
  phase: 'confirm',
  toDefault: false,
  target: '/Volumes/Ext/EmailOps',
  neededBytes: 2048,
  error: null,
};

describe('dataMoveReducer', () => {
  it('asks to confirm the folder the user picked', () => {
    const next = dataMoveReducer(IDLE, {
      type: 'picked',
      toDefault: false,
      target: '/Volumes/Ext/EmailOps',
      neededBytes: 2048,
    });
    expect(next).toEqual(confirming);
  });

  it('goes back to idle when the user cancels', () => {
    expect(dataMoveReducer(confirming, { type: 'cancelled' })).toEqual(IDLE);
  });

  it('starts moving with no progress yet once confirmed', () => {
    expect(dataMoveReducer(confirming, { type: 'confirmed' })).toEqual({
      ...confirming,
      phase: 'moving',
      progress: null,
    });
  });

  it('records progress while moving', () => {
    const moving = dataMoveReducer(confirming, { type: 'confirmed' });
    const next = dataMoveReducer(moving, { type: 'progress', copiedBytes: 10, totalBytes: 20 });
    expect(next).toMatchObject({ phase: 'moving', progress: { copiedBytes: 10, totalBytes: 20 } });
  });

  it('ignores a late progress event once the move is no longer running', () => {
    expect(dataMoveReducer(confirming, { type: 'progress', copiedBytes: 10, totalBytes: 20 })).toBe(confirming);
  });

  it('returns to the confirmation with the error when the move fails, so the user can retry', () => {
    const moving = dataMoveReducer(confirming, { type: 'confirmed' });
    expect(dataMoveReducer(moving, { type: 'failed', error: 'disk full' })).toEqual({
      ...confirming,
      error: 'disk full',
    });
  });

  it('cannot be cancelled while the copy runs', () => {
    const moving = dataMoveReducer(confirming, { type: 'confirmed' });
    expect(dataMoveReducer(moving, { type: 'cancelled' })).toBe(moving);
  });
});
