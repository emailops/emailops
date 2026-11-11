/** Limits on what a composer takes in at once (#274). */
export const MAX_FILES = 32;
export const MAX_FILE_BYTES = 25 * 1024 * 1024;
export const MAX_PENDING_BYTES = 50 * 1024 * 1024;

export type IntakeRefusal = 'empty' | 'tooLarge' | 'tooMany' | 'pendingTooLarge';

export interface IntakePlan {
  /** Indexes into the offered files, in order. */
  accepted: number[];
  refused: { name: string; reason: IntakeRefusal }[];
}

/**
 * Which of `files` a composer that already holds `held` files, and is still
 * reading `pendingBytes`, takes in. Checked before any file is read.
 */
export function planIntake(held: number, pendingBytes: number, files: { name: string; size: number }[]): IntakePlan {
  const plan: IntakePlan = { accepted: [], refused: [] };
  let count = held;
  let pending = pendingBytes;
  files.forEach((file, index) => {
    const reason: IntakeRefusal | null =
      file.size === 0
        ? 'empty'
        : file.size > MAX_FILE_BYTES
          ? 'tooLarge'
          : count >= MAX_FILES
            ? 'tooMany'
            : pending + file.size > MAX_PENDING_BYTES
              ? 'pendingTooLarge'
              : null;
    if (reason) {
      plan.refused.push({ name: file.name, reason });
      return;
    }
    plan.accepted.push(index);
    count += 1;
    pending += file.size;
  });
  return plan;
}
