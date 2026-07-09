import { type RefObject, useLayoutEffect } from 'react';

/** Height cap for the chat input textarea (~9 text rows). Past this the
 *  textarea stops growing and scrolls internally, so a pasted wall of text
 *  can't push the conversation off-screen. */
export const CHAT_INPUT_MAX_HEIGHT_PX = 220;

interface HeightPlan {
  heightPx: number;
  overflowY: 'hidden' | 'auto';
}

/** Pure planner: target inline height for an auto-growing textarea.
 *  `scrollHeightPx` excludes borders while `style.height` includes them
 *  (border-box), so the border must be added back or the content is clipped
 *  by the border width and a phantom scrollbar appears. */
export function planTextareaHeight({
  scrollHeightPx,
  borderPx,
  maxPx,
}: {
  scrollHeightPx: number;
  borderPx: number;
  maxPx: number;
}): HeightPlan {
  const fitPx = scrollHeightPx + borderPx;
  if (fitPx > maxPx) return { heightPx: maxPx, overflowY: 'auto' };
  return { heightPx: fitPx, overflowY: 'hidden' };
}

/** Auto-grow `ref`'s textarea to fit `value`, clamped to `maxPx`.
 *  Runs as a layout effect so the resize is applied before paint (no
 *  one-frame flash of the old height while typing). */
export function useAutoGrow(
  ref: RefObject<HTMLTextAreaElement | null>,
  value: string,
  maxPx: number = CHAT_INPUT_MAX_HEIGHT_PX,
): void {
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    // Collapse first so scrollHeight reflects the current content — without
    // this the measured value can never shrink below the previous height.
    el.style.height = 'auto';
    const plan = planTextareaHeight({
      scrollHeightPx: el.scrollHeight,
      borderPx: el.offsetHeight - el.clientHeight,
      maxPx,
    });
    el.style.height = `${plan.heightPx}px`;
    el.style.overflowY = plan.overflowY;
  }, [ref, value, maxPx]);
}
