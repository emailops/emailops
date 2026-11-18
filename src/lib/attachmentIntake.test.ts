import { describe, expect, it } from 'vitest';
import { MAX_FILE_BYTES, MAX_FILES, MAX_PENDING_BYTES, planIntake } from './attachmentIntake';

const MB = 1024 * 1024;

describe('planIntake', () => {
  it('takes files within the limits, in order', () => {
    expect(
      planIntake(0, 0, [
        { name: 'a.pdf', size: MB },
        { name: 'b.pdf', size: 2 * MB },
      ]),
    ).toEqual({
      accepted: [0, 1],
      refused: [],
    });
  });

  it('refuses an empty file and one over the size limit, and keeps the rest', () => {
    expect(
      planIntake(0, 0, [
        { name: 'empty.txt', size: 0 },
        { name: 'video.mov', size: MAX_FILE_BYTES + 1 },
        { name: 'ok.pdf', size: MB },
      ]),
    ).toEqual({
      accepted: [2],
      refused: [
        { name: 'empty.txt', reason: 'empty' },
        { name: 'video.mov', reason: 'tooLarge' },
      ],
    });
  });

  it('stops at the file count a message can hold', () => {
    const plan = planIntake(MAX_FILES - 1, 0, [
      { name: 'a.pdf', size: MB },
      { name: 'b.pdf', size: MB },
    ]);
    expect(plan.accepted).toEqual([0]);
    expect(plan.refused).toEqual([{ name: 'b.pdf', reason: 'tooMany' }]);
  });

  it('does not read more than the pending limit at once', () => {
    const plan = planIntake(0, MAX_PENDING_BYTES - 2 * MB, [
      { name: 'a.pdf', size: MB },
      { name: 'b.pdf', size: 2 * MB },
    ]);
    expect(plan.accepted).toEqual([0]);
    expect(plan.refused).toEqual([{ name: 'b.pdf', reason: 'pendingTooLarge' }]);
  });
});
