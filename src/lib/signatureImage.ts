// Images in a signature (a logo, a scanned handwritten signature): what the
// "Add image" button accepts and how it shrinks it. Pure decisions plus a thin
// executor whose browser parts (file reader, image decode, canvas) are
// injected. The backend checks the same rules again on save
// (`services/signatures.rs::check_signature_image`).

/** PNG, JPEG, GIF, WebP. Not SVG: it can carry script, and clients refuse it. */
export const SIGNATURE_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'] as const;

/** Largest image a signature may carry (decoded bytes) — the backend's cap. */
export const MAX_SIGNATURE_IMAGE_BYTES = 200 * 1024;

/** Uploads wider than this are scaled down to it. */
export const MAX_SIGNATURE_IMAGE_WIDTH = 600;

/** A source file beyond this is refused before decoding it. */
export const MAX_SIGNATURE_SOURCE_BYTES = 10 * 1024 * 1024;

/** i18n codes: `settings:signatures.imageErrors.<code>`. */
export type SignatureImageError = 'unsupportedType' | 'fileTooLarge' | 'tooLarge' | 'unreadable';

export interface SignatureImageFacts {
  type: string;
  /** Size of the source file. */
  bytes: number;
  width: number;
  height: number;
}

export type SignatureImagePlan =
  | { action: 'accept' }
  | { action: 'downscale'; width: number; height: number; outputType: 'image/png' | 'image/jpeg' }
  | { action: 'reject'; code: SignatureImageError };

const isAllowedType = (type: string) => (SIGNATURE_IMAGE_TYPES as readonly string[]).includes(type);

/**
 * Pure: what to do with an image the user picked. Small enough → as it is
 * (a GIF keeps its animation); too wide or too heavy → redrawn at most
 * `MAX_SIGNATURE_IMAGE_WIDTH` wide, as JPEG when it was one and PNG otherwise
 * (keeps transparency).
 */
export function planSignatureImage(facts: SignatureImageFacts): SignatureImagePlan {
  if (!isAllowedType(facts.type)) return { action: 'reject', code: 'unsupportedType' };
  if (facts.bytes > MAX_SIGNATURE_SOURCE_BYTES) return { action: 'reject', code: 'fileTooLarge' };
  if (facts.width <= 0 || facts.height <= 0) return { action: 'reject', code: 'unreadable' };
  if (facts.width <= MAX_SIGNATURE_IMAGE_WIDTH && facts.bytes <= MAX_SIGNATURE_IMAGE_BYTES) return { action: 'accept' };
  const width = Math.min(facts.width, MAX_SIGNATURE_IMAGE_WIDTH);
  const height = Math.max(1, Math.round((facts.height * width) / facts.width));
  return { action: 'downscale', width, height, outputType: facts.type === 'image/jpeg' ? 'image/jpeg' : 'image/png' };
}

/** Pure: the encoded result still has to fit the cap. */
export function checkEncodedSignatureImage(bytes: number): SignatureImageError | null {
  return bytes > MAX_SIGNATURE_IMAGE_BYTES ? 'tooLarge' : null;
}

/** Decoded size of a base64 data URL. */
export function dataUrlBytes(dataUrl: string): number {
  const payload = dataUrl.slice(dataUrl.indexOf(',') + 1);
  const padding = payload.endsWith('==') ? 2 : payload.endsWith('=') ? 1 : 0;
  return Math.floor((payload.length * 3) / 4) - padding;
}

/** The browser work, injected so the executor is testable without a canvas. */
export interface SignatureImageDeps {
  readDataUrl: (file: File) => Promise<string>;
  measure: (dataUrl: string) => Promise<{ width: number; height: number }>;
  resize: (dataUrl: string, width: number, height: number, type: 'image/png' | 'image/jpeg') => Promise<string>;
}

export type PreparedSignatureImage = { ok: true; dataUrl: string } | { ok: false; code: SignatureImageError };

/** Turn a picked file into the data URL to insert, or the reason it cannot be. */
export async function prepareSignatureImage(file: File, deps: SignatureImageDeps): Promise<PreparedSignatureImage> {
  // Type and size are known without reading the file.
  const early = planSignatureImage({ type: file.type, bytes: file.size, width: 1, height: 1 });
  if (early.action === 'reject') return { ok: false, code: early.code };
  let dataUrl: string;
  let size: { width: number; height: number };
  try {
    dataUrl = await deps.readDataUrl(file);
    size = await deps.measure(dataUrl);
  } catch {
    return { ok: false, code: 'unreadable' };
  }
  const plan = planSignatureImage({ type: file.type, bytes: file.size, ...size });
  if (plan.action === 'reject') return { ok: false, code: plan.code };
  if (plan.action === 'downscale') {
    try {
      dataUrl = await deps.resize(dataUrl, plan.width, plan.height, plan.outputType);
    } catch {
      return { ok: false, code: 'unreadable' };
    }
  }
  const tooLarge = checkEncodedSignatureImage(dataUrlBytes(dataUrl));
  return tooLarge ? { ok: false, code: tooLarge } : { ok: true, dataUrl };
}

/** The real browser dependencies (FileReader, Image, canvas). */
export const browserSignatureImageDeps: SignatureImageDeps = {
  readDataUrl: (file) =>
    new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => (typeof reader.result === 'string' ? resolve(reader.result) : reject(new Error('read')));
      reader.onerror = () => reject(reader.error ?? new Error('read'));
      reader.readAsDataURL(file);
    }),
  measure: (dataUrl) =>
    new Promise((resolve, reject) => {
      const image = new Image();
      image.onload = () => resolve({ width: image.naturalWidth, height: image.naturalHeight });
      image.onerror = () => reject(new Error('decode'));
      image.src = dataUrl;
    }),
  resize: (dataUrl, width, height, type) =>
    new Promise((resolve, reject) => {
      const image = new Image();
      image.onload = () => {
        const canvas = document.createElement('canvas');
        canvas.width = width;
        canvas.height = height;
        const context = canvas.getContext('2d');
        if (!context) {
          reject(new Error('canvas'));
          return;
        }
        if (type === 'image/jpeg') {
          // JPEG has no alpha: paint white first instead of black.
          context.fillStyle = '#ffffff';
          context.fillRect(0, 0, width, height);
        }
        context.drawImage(image, 0, 0, width, height);
        resolve(canvas.toDataURL(type, 0.9));
      };
      image.onerror = () => reject(new Error('decode'));
      image.src = dataUrl;
    }),
};

const escapeAttribute = (value: string) =>
  value.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/** Pure: the paragraph appended to the signature for an added image. */
export function signatureImageHtml(dataUrl: string, alt: string): string {
  return `<p><img src="${escapeAttribute(dataUrl)}" alt="${escapeAttribute(alt)}"></p>`;
}
