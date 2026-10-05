import { bytesToBase64 } from '@/lib/yjsBytes';

/** A picked file's bytes as standard base64, the shape the backend reads. */
export async function fileToBase64(file: Blob): Promise<string> {
  return bytesToBase64(new Uint8Array(await file.arrayBuffer()));
}
