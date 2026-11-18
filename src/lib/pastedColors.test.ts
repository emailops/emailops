import { describe, expect, it } from 'vitest';
import { dropUnreadableColors } from './pastedColors';

function first(html: string, selector: string): HTMLElement {
  const el = new DOMParser()
    .parseFromString(`<body>${dropUnreadableColors(html)}</body>`, 'text/html')
    .body.querySelector<HTMLElement>(selector);
  if (!el) throw new Error(`${selector} not found`);
  return el;
}

describe('dropUnreadableColors', () => {
  it('drops a text colour that does not read on a white page and keeps the other styles', () => {
    const p = first('<p style="color: rgb(218, 218, 218); margin-left: 8px;">pale</p>', 'p');
    expect(p.style.color).toBe('');
    expect(p.style.marginLeft).toBe('8px');
  });

  it('removes the style attribute when the colour was all it had', () => {
    expect(first('<p style="color: #ffffff;">white</p>', 'p').hasAttribute('style')).toBe(false);
  });

  it.each([
    ['black', 'rgb(0, 0, 0)'],
    ['dark grey', 'rgb(55, 65, 81)'],
    ['red', 'rgb(220, 38, 38)'],
    ['blue', 'rgb(37, 99, 235)'],
  ])('keeps %s text on a white page', (_name, color) => {
    expect(first(`<span style="color: ${color};">x</span>`, 'span').style.color).toBe(color);
  });

  it('judges the colour against the background the element itself declares', () => {
    const span = first('<span style="color: rgb(255, 255, 255); background-color: rgb(30, 30, 30);">x</span>', 'span');
    expect(span.style.color).toBe('rgb(255, 255, 255)');
  });

  it('judges the colour against the background of an enclosing pasted element', () => {
    const html =
      '<div style="background-color: rgb(30, 30, 30);"><p><span style="color: rgb(230, 230, 230);">x</span></p></div>';
    expect(first(html, 'span').style.color).toBe('rgb(230, 230, 230)');
  });

  it('takes a bgcolor attribute as the background', () => {
    const html =
      '<table><tbody><tr><td bgcolor="#000000"><span style="color: #ffffff;">x</span></td></tr></tbody></table>';
    expect(first(html, 'span').style.color).not.toBe('');
  });

  it('drops a pale colour on a pale background', () => {
    const html =
      '<p style="background-color: rgb(255, 255, 224);"><span style="color: rgb(250, 250, 240);">x</span></p>';
    expect(first(html, 'span').style.color).toBe('');
    expect(first(html, 'p').style.backgroundColor).toBe('rgb(255, 255, 224)');
  });

  it('drops the colour of a <font> tag that does not read and keeps its face', () => {
    const font = first('<font color="#fafafa" face="Georgia">x</font>', 'font');
    expect(font.hasAttribute('color')).toBe(false);
    expect(font.getAttribute('face')).toBe('Georgia');
  });

  it('leaves a colour it cannot measure as it is', () => {
    expect(first('<span style="color: var(--text-normal);">x</span>', 'span').getAttribute('style')).toBe(
      'color: var(--text-normal);',
    );
  });
});
