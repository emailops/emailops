import { describe, expect, it } from 'vitest';
import { visibleOnboardingSteps } from './onboardingSteps';

describe('visibleOnboardingSteps', () => {
  it('walks every step on a desktop with AI on', () => {
    expect(visibleOnboardingSteps(true, false)).toEqual([1, 2, 3, 4]);
  });

  it('skips the AI backend step when the user opts out of AI', () => {
    expect(visibleOnboardingSteps(false, false)).toEqual([1, 3, 4]);
  });

  it('skips the inbox-layout step on a phone', () => {
    // Step 3 offers split view vs full-width, and a stacked layout forces
    // full-width regardless — so the step is a question whose answer is
    // ignored. Skipping it also drops a screen from first-run on the device
    // with the least patience for them.
    expect(visibleOnboardingSteps(true, true)).toEqual([1, 2, 4]);
  });

  it('skips both the AI backend and the layout step on a phone with AI off', () => {
    expect(visibleOnboardingSteps(false, true)).toEqual([1, 4]);
  });

  it('always keeps the first and last step', () => {
    // Whatever else is dropped, the user must still be able to choose AI and
    // to add an account — without an account the app has nothing to show.
    for (const ai of [true, false]) {
      for (const stacked of [true, false]) {
        const steps = visibleOnboardingSteps(ai, stacked);
        expect(steps[0]).toBe(1);
        expect(steps[steps.length - 1]).toBe(4);
      }
    }
  });
});
