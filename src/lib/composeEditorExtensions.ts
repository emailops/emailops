/**
 * The Tiptap extension list behind `RichTextEditor`, kept free of React so
 * the schema can be round-trip tested headlessly.
 *
 * Besides what the toolbar can produce, the schema has to REPRESENT what a
 * draft written in the provider's own web client carries — tables with their
 * sections, caption and column widths, inline styles, `<sub>` / `<sup>` /
 * `<small>` / `<center>` — or ProseMirror drops it the moment the draft is
 * opened here. The attribute lists follow what the backend keeps on send
 * (`sanitize_outgoing_html`); the editor only carries them, the backend
 * decides what is safe.
 */

import Image from '@tiptap/extension-image';
import Link from '@tiptap/extension-link';
import Paragraph from '@tiptap/extension-paragraph';
import { Table, TableCell, TableHeader, TableRow, TableView } from '@tiptap/extension-table';
import { TextStyle } from '@tiptap/extension-text-style';
import Underline from '@tiptap/extension-underline';
import { DOMSerializer, type Node as ProseMirrorNode } from '@tiptap/pm/model';
import { type Attributes, Extension, type Extensions, Mark, mergeAttributes, Node } from '@tiptap/react';
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

/** `thead` / `tfoot` a row was written in; `null` is the body. */
type RowSection = 'head' | 'foot' | null;

/** The caption of a table: its text and the two attributes the backend keeps. */
interface TableCaption {
  text: string;
  attrs: Record<string, string>;
}

/** The attributes of each `<col>`, in order. */
type TableCols = Record<string, string>[];

function pickAttributes(element: Element, names: string[]): Record<string, string> {
  const picked: Record<string, string> = {};
  for (const name of names) {
    const value = element.getAttribute(name);
    if (value !== null) picked[name] = value;
  }
  return picked;
}

/** A direct child of the table: nested tables have their own. */
function ownChildren(table: Element, tag: string): Element[] {
  return Array.from(table.children).filter((child) => child.tagName === tag);
}

function parseCaption(table: HTMLElement): TableCaption | null {
  const caption = ownChildren(table, 'CAPTION')[0];
  if (!caption) return null;
  return { text: caption.textContent ?? '', attrs: pickAttributes(caption, ['style', 'align']) };
}

function parseCols(table: HTMLElement): TableCols | null {
  const cols = ownChildren(table, 'COLGROUP').flatMap((group) => {
    const own = Array.from(group.children).filter((child) => child.tagName === 'COL');
    // A colgroup without <col> stands for `span` columns itself.
    return (own.length > 0 ? own : [group]).map((col) => pickAttributes(col, ['span', 'width', 'style']));
  });
  return cols.length > 0 ? cols : null;
}

/** How many columns a table has, read off its first row. */
function columnCount(table: ProseMirrorNode): number {
  let count = 0;
  table.firstChild?.forEach((cell) => {
    count += Number(cell.attrs.colspan) || 1;
  });
  return count;
}

function element(tag: string, attributes: Record<string, unknown> = {}): HTMLElement {
  const el = document.createElement(tag);
  for (const [name, value] of Object.entries(attributes)) {
    if (value !== null && value !== undefined) el.setAttribute(name, String(value));
  }
  return el;
}

/**
 * A table as HTML. ProseMirror's table model is a flat list of rows, so the
 * sections cannot be nodes: each row remembers the one it came from and the
 * rows are regrouped here, header first and footer last. The caption and the
 * column widths travel as attributes of the table. A `<colgroup>` that no
 * longer adds up to the table's columns (one was added or removed) is left
 * out rather than written wrong.
 */
function renderTable(node: ProseMirrorNode, attributes: Record<string, unknown>): HTMLElement {
  const table = element('table', attributes);

  const caption = node.attrs.caption as TableCaption | null;
  if (caption) {
    table.appendChild(element('caption', caption.attrs)).textContent = caption.text;
  }

  const cols = node.attrs.cols as TableCols | null;
  if (cols && cols.reduce((sum, col) => sum + (Number(col.span) || 1), 0) === columnCount(node)) {
    const colgroup = table.appendChild(element('colgroup'));
    for (const col of cols) colgroup.appendChild(element('col', col));
  }

  const serializer = DOMSerializer.fromSchema(node.type.schema);
  const sections: [RowSection, string][] = [
    ['head', 'thead'],
    [null, 'tbody'],
    ['foot', 'tfoot'],
  ];
  for (const [section, tag] of sections) {
    const rows: ProseMirrorNode[] = [];
    node.forEach((row) => {
      if ((row.attrs.section as RowSection) === section) rows.push(row);
    });
    if (rows.length === 0) continue;
    const group = table.appendChild(element(tag));
    for (const row of rows) group.appendChild(serializer.serializeNode(row));
  }
  return table;
}

/**
 * The stock editing view, plus the caption: it is not part of the document
 * (see `renderTable`), so it is shown above the rows, read-only.
 */
class ComposeTableView extends TableView {
  private caption: HTMLElement | null = null;

  constructor(...args: ConstructorParameters<typeof TableView>) {
    super(...args);
    this.showCaption(args[0]);
  }

  update(node: ProseMirrorNode): boolean {
    if (!super.update(node)) return false;
    this.showCaption(node);
    return true;
  }

  private showCaption(node: ProseMirrorNode): void {
    const caption = node.attrs.caption as TableCaption | null;
    this.caption?.remove();
    this.caption = null;
    if (!caption) return;
    this.caption = element('caption', { ...caption.attrs, contenteditable: 'false' });
    this.caption.textContent = caption.text;
    this.table.prepend(this.caption);
  }
}

/** A mark for an inline tag that carries nothing but itself. */
function inlineTagMark(name: string, tag: string): Mark {
  return Mark.create({
    name,
    parseHTML() {
      return [{ tag }];
    },
    renderHTML() {
      return [tag, 0];
    },
  });
}

/**
 * `<center>` as a block around other blocks, written back as `<center>`:
 * newsletters centre whole tables with it. Text directly inside it becomes a
 * paragraph inside it, which looks the same.
 */
const Center = Node.create({
  name: 'center',
  group: 'block',
  content: 'block+',
  defining: true,
  parseHTML() {
    return [{ tag: 'center' }];
  },
  renderHTML() {
    return ['center', 0];
  },
});

/**
 * The account signature a composer inserted (`src/lib/signature.ts`), kept as
 * one block so it can be swapped when the From account changes and kept when
 * an AI draft replaces the text. Ranked above the paragraph rule, which would
 * otherwise read a text-only `<div>` as a plain paragraph.
 */
const Signature = Node.create({
  name: 'signature',
  group: 'block',
  content: 'block+',
  defining: true,
  parseHTML() {
    return [{ tag: 'div[data-emailops-signature]', priority: 60 }];
  },
  renderHTML() {
    return ['div', { 'data-emailops-signature': '' }, 0];
  },
});

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
  // What the table was written with does: see `renderTable`.
  Table.extend({
    addAttributes() {
      return {
        ...this.parent?.(),
        caption: { default: null, rendered: false, parseHTML: parseCaption },
        cols: { default: null, rendered: false, parseHTML: parseCols },
      };
    },
    parseHTML() {
      // The caption is read into an attribute; as content it would become a row.
      return [{ tag: 'table' }, { tag: 'caption', ignore: true }];
    },
    renderHTML({ node, HTMLAttributes }) {
      return renderTable(node, mergeAttributes(this.options.HTMLAttributes, HTMLAttributes));
    },
  }).configure({ View: ComposeTableView }),
  TableRow.extend({
    addAttributes() {
      return {
        ...this.parent?.(),
        section: {
          default: null,
          rendered: false,
          parseHTML: (row): RowSection =>
            row.parentElement?.tagName === 'THEAD' ? 'head' : row.parentElement?.tagName === 'TFOOT' ? 'foot' : null,
        },
      };
    },
  }),
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
  inlineTagMark('subscript', 'sub'),
  inlineTagMark('superscript', 'sup'),
  inlineTagMark('small', 'small'),
  Center,
  Signature,
  PreservedAttributes,
];
