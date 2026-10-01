import { Editor } from '@tiptap/core';
import Image from '@tiptap/extension-image';
import StarterKit from '@tiptap/starter-kit';
import { describe, expect, it } from 'vitest';
import {
  applySignature,
  bodyWithoutSignature,
  clampSignatureWidth,
  hasSignature,
  parseSignatureImage,
  SIGNATURE_IMAGE_DEFAULT_WIDTH,
  SIGNATURE_IMAGE_MAX_BYTES,
  SIGNATURE_IMAGE_MAX_WIDTH,
  SIGNATURE_IMAGE_MIN_WIDTH,
  serializeSignatureImage,
  signatureImagePrefKey,
  signaturePrefKey,
  signatureToHtml,
  stripSignature,
} from './signature';

const PNG =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=';

const SIG = 'Maxime Theriault\nFou d’la bouffe\n418 555-0100';

/** What the composer holds after Tiptap has normalized the HTML. */
function throughEditor(html: string): string {
  // Same image config as RichTextEditor, so width and data: URLs behave alike.
  const editor = new Editor({ extensions: [StarterKit, Image.configure({ allowBase64: true })], content: html });
  const out = editor.getHTML();
  editor.destroy();
  return out;
}

describe('signaturePrefKey', () => {
  it('is namespaced per account', () => {
    expect(signaturePrefKey('acc-1')).toBe('signature:acc-1');
  });
});

describe('signatureToHtml', () => {
  it('is empty for a blank signature', () => {
    expect(signatureToHtml('')).toBe('');
    expect(signatureToHtml('  \n \n')).toBe('');
  });

  it('puts the delimiter first and one paragraph per line', () => {
    expect(signatureToHtml('A\nB')).toBe('<p>--</p><p>A</p><p>B</p>');
  });

  it('keeps blank lines and escapes HTML', () => {
    expect(signatureToHtml('A\n\n<b>x</b> & y')).toBe('<p>--</p><p>A</p><p></p><p>&lt;b&gt;x&lt;/b&gt; &amp; y</p>');
  });
});

describe('applySignature', () => {
  it('gives an empty new message a line to type on, then the signature', () => {
    expect(applySignature('', 'A')).toBe('<p></p><p>--</p><p>A</p>');
    expect(applySignature('<p></p>', 'A')).toBe('<p></p><p>--</p><p>A</p>');
  });

  it('appends after text the user already typed', () => {
    expect(applySignature('<p>Hello</p>', 'A')).toBe('<p>Hello</p><p>--</p><p>A</p>');
  });

  it('goes between the reply and the quoted original', () => {
    const html = '<p>Thanks!</p><blockquote><p>original</p></blockquote>';
    expect(applySignature(html, 'A')).toBe('<p>Thanks!</p><p>--</p><p>A</p><blockquote><p>original</p></blockquote>');
  });

  it('goes above a forwarded message', () => {
    const html = '<p>---------- Forwarded message ---------</p><p>From: x</p>';
    expect(applySignature(html, 'A')).toBe(`<p></p><p>--</p><p>A</p>${html}`);
  });

  it('never touches the original sender’s "--" signature inside a forward or quote', () => {
    const forward =
      '<p>---------- Forwarded message ----------</p><p>From: bob@x.com</p><p>Hello</p><p>--</p><p>Bob, Acme</p>';
    const withMine = applySignature(forward, 'Max');
    expect(withMine).toBe(`<p></p><p>--</p><p>Max</p>${forward}`);
    expect(stripSignature(withMine)).toBe(`<p></p>${forward}`);
    expect(applySignature(withMine, '')).toBe(`<p></p>${forward}`);

    const reply = '<p>Ok</p><blockquote><p>Hi</p><p>--</p><p>Bob</p></blockquote>';
    expect(hasSignature(reply)).toBe(false);
    expect(applySignature(reply, 'Max')).toBe(
      '<p>Ok</p><p>--</p><p>Max</p><blockquote><p>Hi</p><p>--</p><p>Bob</p></blockquote>',
    );
  });

  it('replaces the previous signature instead of stacking a second one', () => {
    const once = applySignature('<p>Hi</p>', 'Old');
    expect(applySignature(once, 'New')).toBe('<p>Hi</p><p>--</p><p>New</p>');
  });

  it('removes the signature when switching to an account without one', () => {
    const once = applySignature('<p>Hi</p><blockquote><p>q</p></blockquote>', 'Old');
    expect(applySignature(once, '')).toBe('<p>Hi</p><blockquote><p>q</p></blockquote>');
  });

  it('leaves a body without signature untouched when there is none to add', () => {
    expect(applySignature('<p>Hi</p>', '')).toBe('<p>Hi</p>');
  });
});

describe('stripSignature / hasSignature / bodyWithoutSignature', () => {
  it('detects and strips only the signature, keeping the quote', () => {
    const html = applySignature('<p>Hi</p><blockquote><p>q</p></blockquote>', SIG);
    expect(hasSignature(html)).toBe(true);
    expect(stripSignature(html)).toBe('<p>Hi</p><blockquote><p>q</p></blockquote>');
    expect(hasSignature('<p>Hi</p>')).toBe(false);
  });

  it('treats a composer that only holds the signature as empty', () => {
    expect(bodyWithoutSignature(applySignature('', SIG))).toBe('');
  });

  it('returns what the user typed, without the signature', () => {
    expect(bodyWithoutSignature(applySignature('<p>Please call me</p>', SIG))).toBe('Please call me');
  });
});

describe('signature image', () => {
  it('has its own preference key', () => {
    expect(signatureImagePrefKey('acc-1')).toBe('signature_image:acc-1');
  });

  it('is rendered after the text at the chosen width', () => {
    expect(signatureToHtml({ text: 'Max', image: { src: PNG, width: 200 } })).toBe(
      `<p>--</p><p>Max</p><img src="${PNG}" alt="Signature" width="200">`,
    );
  });

  it('can be the whole signature, with no text', () => {
    expect(applySignature('<p>Hi</p>', { text: '', image: { src: PNG, width: 120 } })).toBe(
      `<p>Hi</p><p>--</p><img src="${PNG}" alt="Signature" width="120">`,
    );
  });

  it('is swapped and removed together with the text', () => {
    const once = applySignature('<p>Hi</p><blockquote><p>q</p></blockquote>', {
      text: 'Max',
      image: { src: PNG, width: 120 },
    });
    expect(applySignature(once, 'Other')).toBe('<p>Hi</p><p>--</p><p>Other</p><blockquote><p>q</p></blockquote>');
    expect(applySignature(once, '')).toBe('<p>Hi</p><blockquote><p>q</p></blockquote>');
  });

  it('keeps its width through the editor', () => {
    const inEditor = throughEditor(applySignature('<p>Hi</p>', { text: 'Max', image: { src: PNG, width: 180 } }));
    expect(inEditor).toContain('width="180"');
    expect(bodyWithoutSignature(inEditor)).toBe('Hi');
    expect(throughEditor(applySignature(inEditor, 'Max'))).not.toContain('<img');
  });

  it('only accepts image data URLs of a bounded size', () => {
    expect(signatureToHtml({ text: '', image: { src: 'https://tracker.example/pixel.png', width: 100 } })).toBe(
      '<p>--</p>',
    );
    expect(signatureToHtml({ text: '', image: { src: 'data:text/html;base64,PHNjcmlwdD4=', width: 100 } })).toBe(
      '<p>--</p>',
    );
    const huge = `data:image/png;base64,${'A'.repeat(SIGNATURE_IMAGE_MAX_BYTES)}`;
    expect(parseSignatureImage(JSON.stringify({ src: huge, width: 100 }))).toBeNull();
  });

  it('clamps the width into the allowed range', () => {
    expect(clampSignatureWidth(5)).toBe(SIGNATURE_IMAGE_MIN_WIDTH);
    expect(clampSignatureWidth(10_000)).toBe(SIGNATURE_IMAGE_MAX_WIDTH);
    expect(clampSignatureWidth(Number.NaN)).toBe(SIGNATURE_IMAGE_DEFAULT_WIDTH);
    expect(clampSignatureWidth(150.6)).toBe(151);
  });

  it('round-trips through its preference, and reads junk as no image', () => {
    const stored = serializeSignatureImage({ src: PNG, width: 9999 });
    expect(parseSignatureImage(stored)).toEqual({ src: PNG, width: SIGNATURE_IMAGE_MAX_WIDTH });
    expect(serializeSignatureImage(null)).toBe('');
    for (const junk of ['', null, 'not json', '{"src":1}', '{"width":100}']) {
      expect(parseSignatureImage(junk)).toBeNull();
    }
  });
});

describe('survives the Tiptap editor', () => {
  it('is still found, swapped and stripped after the editor normalizes it', () => {
    const inEditor = throughEditor(applySignature('<p>Hi</p><blockquote><p>q</p></blockquote>', SIG));
    expect(hasSignature(inEditor)).toBe(true);
    expect(bodyWithoutSignature(inEditor)).not.toContain('Maxime');
    expect(bodyWithoutSignature(inEditor)).toContain('Hi');
    const swapped = throughEditor(applySignature(inEditor, 'Other'));
    expect(swapped).toContain('<p>Other</p>');
    expect(swapped).not.toContain('Maxime');
    expect(swapped.match(/<p>--<\/p>/g)).toHaveLength(1);
    expect(swapped).toContain('<blockquote><p>q</p></blockquote>');
  });
});
