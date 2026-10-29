/** Zoom levels the toolbar steps through, as scale factors (1 = 100%). */
export const ZOOM_STEPS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3] as const;
export const MIN_ZOOM = ZOOM_STEPS[0];
export const MAX_ZOOM = ZOOM_STEPS[ZOOM_STEPS.length - 1];

/** Fit the page to the pane width, re-fitting as the pane resizes. */
export const FIT_WIDTH = 'fit-width';

/** Either a fixed scale or fit-to-width. */
export type PdfZoom = number | typeof FIT_WIDTH;

export function effectiveScale(zoom: PdfZoom, fitScale: number): number {
  return zoom === FIT_WIDTH ? fitScale : zoom;
}

export function zoomIn(zoom: PdfZoom, fitScale: number): number {
  const current = effectiveScale(zoom, fitScale);
  return ZOOM_STEPS.find((step) => step > current) ?? MAX_ZOOM;
}

export function zoomOut(zoom: PdfZoom, fitScale: number): number {
  const current = effectiveScale(zoom, fitScale);
  return [...ZOOM_STEPS].reverse().find((step) => step < current) ?? MIN_ZOOM;
}

export function canZoomIn(zoom: PdfZoom, fitScale: number): boolean {
  return effectiveScale(zoom, fitScale) < MAX_ZOOM;
}

export function canZoomOut(zoom: PdfZoom, fitScale: number): boolean {
  return effectiveScale(zoom, fitScale) > MIN_ZOOM;
}

/**
 * The scale at which a page of `pageWidth` (CSS px at 100%) fills
 * `containerWidth` less `padding`, kept inside the zoom range. 100% until both
 * widths are known (before layout, or before the first page loads).
 */
export function fitWidthScale(pageWidth: number, containerWidth: number, padding: number): number {
  const available = containerWidth - padding;
  if (pageWidth <= 0 || available <= 0) return 1;
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, available / pageWidth));
}

/** Cmd/Ctrl + `=`/`+`, `-`, `0` — the browser's zoom shortcuts. */
export function zoomKeyAction(e: { key: string; metaKey: boolean; ctrlKey: boolean }): 'in' | 'out' | 'fit' | null {
  if (!e.metaKey && !e.ctrlKey) return null;
  if (e.key === '=' || e.key === '+') return 'in';
  if (e.key === '-') return 'out';
  if (e.key === '0') return 'fit';
  return null;
}
