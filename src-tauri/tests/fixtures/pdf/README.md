# PDF fixtures

## `missing_tounicode_glyph.pdf`

Three lines of synthetic lab-report text, printed with headless Chrome (Skia),
so the font is an embedded TrueType subset (`Type0` / `Identity-H` /
`CIDFontType2`), the same shape Chrome-printed reports have. The font is Noto
Sans Regular (SIL Open Font License 1.1, which allows embedding).

After printing, the `ToUnicode` entry for "í" (`<00AF> <00ED>`) was removed
from the font's CMap, so the text layer alone cannot say which character that
glyph draws; only the embedded font can. `services::pdf_text` must still read
"Hematíes".

To rebuild it: print an HTML page with the three lines in a Noto Sans
`@font-face` via `chrome --headless --print-to-pdf`, then delete the `00ED`
line from the font's `ToUnicode` stream (and decrement its `beginbfchar`
count) with pikepdf.
