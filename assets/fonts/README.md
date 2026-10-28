The DejaVu font files come from the pinned [Matplotlib v3.10.8 distribution](https://github.com/matplotlib/matplotlib/tree/v3.10.8/lib/matplotlib/mpl-data/fonts/ttf). The files are unmodified. `FONT-LICENSE.txt` contains their redistribution notices.

The Liberation Sans, Liberation Serif and Liberation Mono files are the unmodified [Liberation Fonts 2.1.5](https://github.com/liberationfonts/liberation-fonts/releases/tag/2.1.5) TrueType release (name table `Version 2.1.5`, modified 2021-09-30), copied from the fonts bundled with LibreOffice. They are full fonts of 2,602–2,622 glyphs (Sans and Serif) and 2,423–2,425 glyphs (Mono), not subsets. `LIBERATION-LICENSE.txt` is the upstream SIL Open Font License 1.1 notice, as shipped with pdfjs-dist 5.6.205. The 1.07.4 Liberation Sans subsets inside pdfjs-dist are licensed differently and are not used. Each face has the same advance widths as the matching Arial, Times New Roman or Courier New face for all 651 WGL4 characters both cover, so line breaks and glyph positions match text laid out with those faces.

`SHA256SUMS` records the hashes of every font file.

Liberation Sans, Liberation Serif, Liberation Mono, DejaVu Sans, DejaVu Serif and DejaVu Sans Mono each include regular, bold, italic/oblique and bold italic/oblique faces. The runtime uses these assets directly and does not discover or load platform fonts.
