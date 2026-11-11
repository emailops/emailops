import { generateJSON, getSchema } from '@tiptap/core';
import { describe, expect, it } from 'vitest';
import { docSchemaExtensions } from './docSchema';

function linkAttrs(html: string): Record<string, unknown> {
  const json = generateJSON(html, docSchemaExtensions);
  return json.content?.[0]?.content?.[0]?.marks?.[0]?.attrs ?? {};
}

describe('docSchemaExtensions', () => {
  it.each([
    ['imported with one', '<p><a href="https://example.com" target="_top">x</a></p>'],
    ['written without one', '<p><a href="https://example.com">x</a></p>'],
  ])('stores no target on a link %s', (_how, html) => {
    const attrs = linkAttrs(html);

    expect(attrs.href).toBe('https://example.com');
    expect(attrs).not.toHaveProperty('target');
  });

  it('drops a target an existing document already stored', () => {
    const mark = getSchema(docSchemaExtensions).marks.link.create({ href: 'https://example.com', target: '_top' });

    expect(mark.attrs).not.toHaveProperty('target');
  });
});
