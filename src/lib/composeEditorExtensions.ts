/**
 * The Tiptap extension list behind `RichTextEditor`, kept free of React so
 * the schema can be round-trip tested headlessly.
 *
 * Besides what the toolbar can produce, the schema has to REPRESENT what a
 * draft written in the provider's own web client carries — tables and inline
 * styles — or ProseMirror drops it the moment the draft is opened here. The
 * attribute lists follow what the backend keeps on send
 * (`sanitize_outgoing_html`); the editor only carries them, the backend
 * decides what is safe.
 */

import Image from '@tiptap/extension-image';
import Link from '@tiptap/extension-link';
import Paragraph from '@tiptap/extension-paragraph';
import { Table, TableCell, TableHeader, TableRow } from '@tiptap/extension-table';
import { TextStyle } from '@tiptap/extension-text-style';
import Underline from '@tiptap/extension-underline';
import { type Attributes, Extension, type Extensions, mergeAttributes } from '@tiptap/react';
import StarterKit from '@tiptap/starter-kit';

/** Attributes carried verbatim: parsed from and rendered to the same HTML attribute. */
function verbatim(...names: string[]): Attributes {
  return Object.fromEntries(names.map((name) => [name, { default: null }]));
}

const PreservedAttributes = Extension.create({
  name: 'preservedAttributes',
  addGlobalAttributes() {
    return [
      {
        types: ['paragraph', 'heading', 'blockquote', 'bulletList', 'orderedList', 'listItem', 'table', 'tableRow'],
        attributes: verbatim('style'),
      },
      { types: ['paragraph', 'heading'], attributes: verbatim('align') },
      {
        types: ['table'],
        attributes: verbatim('width', 'height', 'align', 'bgcolor', 'border', 'cellpadding', 'cellspacing'),
      },
      { types: ['tableRow'], attributes: verbatim('height', 'align', 'valign', 'bgcolor') },
    ];
  },
});

/**
 * Cells replace two stock attributes: `align` (stock re-renders it as a
 * `text-align` style, which would duplicate the verbatim `style`) and
 * `colwidth` (stock renders a non-HTML `colwidth` attribute).
 */
function cellAttributes(stock: Attributes | undefined): Attributes {
  return {
    ...stock,
    colwidth: { default: null, rendered: false },
    ...verbatim('style', 'width', 'height', 'align', 'valign', 'bgcolor'),
  };
}

/** The `<font size>` scale, 1–7, as CSS keywords. */
const FONT_SIZES = ['x-small', 'small', 'medium', 'large', 'x-large', 'xx-large', 'xxx-large'];

/** The inline style of a `<span>`, or the one a `<font>` tag's attributes stand for. */
function ownTextStyle(element: Element): string {
  if (element.tagName !== 'FONT') return (element.getAttribute('style') ?? '').trim().replace(/;$/, '');
  const color = element.getAttribute('color');
  const face = element.getAttribute('face');
  const size = FONT_SIZES[Number(element.getAttribute('size')) - 1];
  return [color && `color: ${color}`, face && `font-family: ${face}`, size && `font-size: ${size}`]
    .filter(Boolean)
    .join('; ');
}

/**
 * A mark cannot nest inside itself, so text inside nested styled spans gets
 * one mark carrying the outer styles followed by the inner ones.
 */
function inheritedTextStyle(element: HTMLElement): string | null {
  const styles: string[] = [];
  for (let el: Element | null = element; el; el = el.parentElement?.closest('span, font') ?? null) {
    styles.unshift(ownTextStyle(el));
  }
  return styles.filter(Boolean).join('; ') || null;
}

export const composeEditorExtensions: Extensions = [
  StarterKit.configure({
    // We want StarterKit defaults but explicit about a few things.
    heading: { levels: [1, 2, 3] },
    // StarterKit 3 bundles these; we register our own configured copies below.
    link: false,
    underline: false,
    paragraph: false,
  }),
  // Webmail clients write lines as `<div>`s. ProseMirror already turns a
  // text-only `<div>` into a paragraph; matching it explicitly lets the
  // paragraph keep that div's `style` (alignment, mostly).
  Paragraph.extend({
    parseHTML() {
      return [
        { tag: 'p' },
        {
          tag: 'div',
          getAttrs: (element) =>
            element.querySelector('div, p, table, ul, ol, blockquote, pre, hr, h1, h2, h3, h4, h5, h6') ? false : null,
        },
      ];
    },
  }),
  Underline,
  Link.configure({
    openOnClick: false,
    autolink: true,
    HTMLAttributes: { rel: 'noopener noreferrer' },
  }),
  // Inline images. Tiptap stores them as data: URLs until we extract
  // them at send time via `prepareOutgoingHtml`.
  Image.configure({
    allowBase64: true,
    HTMLAttributes: { class: 'max-w-full h-auto rounded' },
  }),
  // Stock rendering adds a `<colgroup>` of `min-width` columns and a
  // `min-width` style for its resize UI; none of that belongs in an email.
  Table.extend({
    renderHTML({ HTMLAttributes }) {
      return ['table', mergeAttributes(this.options.HTMLAttributes, HTMLAttributes), ['tbody', 0]];
    },
  }),
  TableRow,
  TableCell.extend({
    addAttributes() {
      return cellAttributes(this.parent?.());
    },
  }),
  TableHeader.extend({
    addAttributes() {
      return cellAttributes(this.parent?.());
    },
  }),
  TextStyle.extend({
    addAttributes() {
      return { style: { default: null, parseHTML: inheritedTextStyle } };
    },
    parseHTML() {
      return [{ tag: 'span', getAttrs: (element) => (element.hasAttribute('style') ? null : false) }, { tag: 'font' }];
    },
  }),
  PreservedAttributes,
];
