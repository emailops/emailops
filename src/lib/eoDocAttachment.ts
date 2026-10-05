import type { EmailAttachment } from '@/lib/api';
import type { SharedDoc } from '@/types';

/** Mirrors `services::shared_docs::DOC_REF_MIME`: an EO Doc attached to an
 *  email travels to the backend as this placeholder (data = the document id),
 *  and is swapped for the document itself only when the email goes out. */
export const DOC_REF_MIME = 'application/vnd.emailops.doc-ref';

export function eoDocAttachment(doc: SharedDoc): EmailAttachment {
  return { filename: `${doc.title}.eodoc`, mimeType: DOC_REF_MIME, data: btoa(doc.id) };
}

export function isEoDocAttachment(att: Pick<EmailAttachment, 'mimeType'>): boolean {
  return att.mimeType === DOC_REF_MIME;
}
