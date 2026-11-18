/**
 * A Word (.docx) attachment as HTML, for the read-only viewer.
 *
 * docx-preview lays the document out as Word does (styles, tables, page size),
 * but it builds its nodes with the app's own `document` and would attach its
 * `<style>` and links straight into the app. The nodes are never attached here:
 * they are serialized, and the caller renders the HTML through `EmailBody`,
 * which sanitizes it and shows it in the sandboxed email frame like any other
 * untrusted markup.
 */
import type { Options } from 'docx-preview';

const PREVIEW_OPTIONS: Partial<Options> = {
  // The frame has a null origin: a blob: URL minted here would not load there.
  useBase64URL: true,
  // An altChunk is a whole embedded HTML page, which docx-preview frames itself.
  renderAltChunks: false,
};

export async function docxToPreviewHtml(data: ArrayBuffer): Promise<string> {
  const { parseAsync, renderDocument } = await import('docx-preview');
  const document = await parseAsync(data, PREVIEW_OPTIONS);
  const nodes = await renderDocument(document, PREVIEW_OPTIONS);
  // Only elements: the library also returns HTML comments (one labels its
  // stylesheet), which must not reach the page as text.
  return nodes.map((node) => (node instanceof Element ? node.outerHTML : '')).join('');
}
