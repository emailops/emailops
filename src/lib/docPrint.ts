import DOMPurify from 'dompurify';

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/**
 * Pure: what a document's PDF shows — its title, then its content. The
 * content can come from other people (it is merged from their mail), so it is
 * sanitized before it is put into the app's own page for printing.
 */
export function printableHtml(title: string, contentHtml: string): string {
  const body = DOMPurify.sanitize(contentHtml, { USE_PROFILES: { html: true } });
  return `<h1>${escapeHtml(title)}</h1>${body}`;
}

/** The element holding what is printed: hidden on screen, the only thing a
 *  print shows (see `index.css`). */
export const PRINT_ROOT_ID = 'eo-print-root';

/**
 * Open the system print dialog on the document alone, where "Save as PDF"
 * writes the file. The window takes the document's title meanwhile, so the
 * PDF is named after it. The printed copy stays in the page (hidden) because
 * the macOS dialog renders it after `print()` has returned.
 */
export async function printDocument(title: string, contentHtml: string): Promise<void> {
  let root = document.getElementById(PRINT_ROOT_ID);
  if (!root) {
    root = document.createElement('div');
    root.id = PRINT_ROOT_ID;
    document.body.appendChild(root);
  }
  root.innerHTML = printableHtml(title, contentHtml);
  const appTitle = document.title;
  document.title = title;
  try {
    // Tauri's macOS print returns a promise; the browser's returns nothing.
    await (window.print() as unknown as Promise<void> | undefined);
  } finally {
    document.title = appTitle;
  }
}
