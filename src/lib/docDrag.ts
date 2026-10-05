// Drag-and-drop contract between EO Docs rows (drag sources) and folders or
// breadcrumb entries (drop targets). A custom MIME type keeps foreign drags
// (files, text selections, emails) from ever looking like a document move.

export const DOC_DRAG_MIME = 'application/x-emailops-eodoc';

export function writeDocDrag(dataTransfer: DataTransfer, docId: string): void {
  dataTransfer.setData(DOC_DRAG_MIME, docId);
  dataTransfer.effectAllowed = 'move';
}

/** The dragged document's id; null for a drag that is not ours. */
export function readDocDrag(dataTransfer: DataTransfer): string | null {
  return dataTransfer.getData(DOC_DRAG_MIME) || null;
}

/** True when a dragover carries a document (its data is not readable until the drop). */
export function isDocDrag(dataTransfer: DataTransfer): boolean {
  return Array.from(dataTransfer.types).includes(DOC_DRAG_MIME);
}
