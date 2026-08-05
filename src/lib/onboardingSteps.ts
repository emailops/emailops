/** The first-run wizard's screens, in order. */
export type OnboardingStep = 1 | 2 | 3 | 4;

/**
 * Which onboarding screens this run actually shows.
 *
 * Two screens are conditional, and both are dropped for the same reason —
 * asking a question whose answer cannot matter:
 *
 *  - **2, AI backend** — irrelevant once the user has turned AI off.
 *  - **3, inbox layout** — a stacked layout is forced to full-width, so the
 *    split-view option would be selectable and then ignored.
 *
 * Numbering stays absolute (the wizard's `step` state is a screen id, not an
 * index); the header derives its "X of N" from this list's length, so a
 * skipped screen never leaves a gap like "1, 3, 4" in the visible count.
 */
export function visibleOnboardingSteps(aiEnabled: boolean, isStacked: boolean): OnboardingStep[] {
  const steps: OnboardingStep[] = [1];
  if (aiEnabled) steps.push(2);
  if (!isStacked) steps.push(3);
  steps.push(4);
  return steps;
}
