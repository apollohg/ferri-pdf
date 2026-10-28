from pathlib import Path
import tempfile
import unittest

from comparison import analyse, text_lines
from pdf_content import Name, PdfError, Ref, inspect, serialize
from test_support import write_pdf_objects


class ReplacementTextTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def pdf(self, content, *, form=None, properties=None, unicode_map=None):
        font = {'Type': Name('Font'), 'Subtype': Name('Type1'), 'BaseFont': Name('Helvetica'),
                'Encoding': Name('WinAnsiEncoding'), 'FirstChar': 0, 'LastChar': 255,
                'Widths': [600] * 256}
        resources = {'Font': {'F1': Ref(3, 0)}}
        if form is not None:
            resources['XObject'] = {'Nested': Ref(6, 0)}
        if properties is not None:
            resources['Properties'] = {'Replacement': Ref(7 if form is not None else 6, 0)}
        stream = lambda data, **entries: serialize({'Length': len(data), **entries}) + b'\nstream\n' + data + b'\nendstream'
        objects = [serialize({'Type': Name('Catalog'), 'Pages': Ref(2, 0)}),
                   serialize({'Type': Name('Pages'), 'Count': 1, 'Kids': [Ref(4, 0)]}),
                   serialize(font),
                   serialize({'Type': Name('Page'), 'Parent': Ref(2, 0), 'MediaBox': [0, 0, 200, 200],
                              'Resources': resources, 'Contents': Ref(5, 0)}), stream(content)]
        if form is not None:
            objects.append(stream(form, Type=Name('XObject'), Subtype=Name('Form'), BBox=[0, 0, 200, 200],
                                  Resources={'Font': {'F1': Ref(3, 0)}}))
        if properties is not None:
            objects.append(serialize(properties))
        if unicode_map is not None:
            font['ToUnicode'] = Ref(len(objects) + 1, 0)
            objects[2] = serialize(font)
            entries = b'\n'.join(f'<{code:02x}> <{text.encode("utf-16-be").hex()}>'.encode()
                                 for code, text in unicode_map.items())
            cmap = (b'/CIDInit /ProcSet findresource begin 12 dict begin begincmap '
                    b'/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def '
                    b'/CMapName /Probe def /CMapType 2 def '
                    b'1 begincodespacerange <00> <ff> endcodespacerange '
                    + str(len(unicode_map)).encode() + b' beginbfchar\n' + entries
                    + b'\nendbfchar endcmap CMapName currentdict /CMap defineresource pop end end')
            objects.append(stream(cmap))
        return write_pdf_objects(self.root/'probe.pdf', objects)

    @staticmethod
    def paint(text):
        return b'BT /F1 12 Tf 1 0 0 1 20 100 Tm ' + serialize(text.encode()) + b' Tj ET '

    def assert_spans(self, path, expected):
        result = inspect(path)
        spans = result['replacement_text']
        actual = [(span.page, span.text, ''.join(glyph.text for glyph in
                   result['glyphs'][span.glyph_start:span.glyph_end])) for span in spans]
        self.assertEqual(actual, expected)
        return result

    def test_unicode_replacement_binds_exact_painted_range_without_rewriting_glyphs(self):
        logical = 'X العربية'
        properties = serialize({'ActualText': b'\xfe\xff' + logical.encode('utf-16-be')})
        content = self.paint('Before') + b'/Span ' + properties + b' BDC ' + self.paint('Visual') + b'EMC ' + self.paint('After')
        result = self.assert_spans(self.pdf(content), [(1, logical, 'Visual')])
        self.assertEqual(''.join(g.text for g in result['glyphs']), 'BeforeVisualAfter')

    def test_outer_replacement_owns_nested_marked_content_and_form_glyphs(self):
        form = b'/Span << /ActualText (Inner) >> BDC ' + self.paint('B') + b'EMC'
        content = b'/Span << /ActualText (Outer) >> BDC /Artifact BMC ' + self.paint('A') + b'/Nested Do EMC ' + self.paint('C') + b'EMC'
        self.assert_spans(self.pdf(content, form=form), [(1, 'Outer', 'ABC')])

    def test_reused_form_creates_distinct_replacement_ranges(self):
        form = b'/Span << /ActualText (Logical) >> BDC ' + self.paint('Visual') + b'EMC'
        self.assert_spans(self.pdf(b'/Nested Do /Nested Do', form=form),
                          [(1, 'Logical', 'Visual'), (1, 'Logical', 'Visual')])

    def test_named_indirect_properties_and_empty_spans_are_preserved(self):
        content = b'/Span /Replacement BDC ' + self.paint('A') + b'EMC /Span << /ActualText () >> BDC EMC'
        self.assert_spans(self.pdf(content, properties={'ActualText': b'Logical'}),
                          [(1, 'Logical', 'A'), (1, '', '')])

    def test_unbalanced_or_invalid_marked_content_is_not_silently_accepted(self):
        for content in [b'EMC', b'/Span BMC', b'/Span << /ActualText (Open) >> BDC',
                        b'/Span /Missing BDC EMC', b'/Span << /ActualText 42 >> BDC EMC']:
            with self.subTest(content=content), self.assertRaises(PdfError):
                inspect(self.pdf(content))

    def test_form_cannot_close_its_callers_marked_content(self):
        with self.assertRaises(PdfError):
            inspect(self.pdf(b'/Span << /ActualText (Outer) >> BDC /Nested Do EMC', form=b'EMC'))

    def test_logical_bidi_replacement_matches_painted_visual_order(self):
        for logical, visual in [('X עברית', 'X תירבע'), ('X العربية', 'X ةيبرعلا'),
                                ('X אב 12 גד', 'X דג 12 בא'),
                                ('X \u2067אב 12\u2069 Y', 'X 12 בא Y')]:
            with self.subTest(logical=logical):
                mapping = dict(enumerate(visual, start=65))
                paint = self.paint(''.join(chr(code) for code in mapping))
                reference = analyse(self.pdf(paint, unicode_map=mapping))['pages']
                properties = serialize({'ActualText': b'\xfe\xff' + logical.encode('utf-16-be')})
                candidate = analyse(self.pdf(b'/Span ' + properties + b' BDC ' + paint + b'EMC',
                                             unicode_map=mapping))['pages']
                self.assertTrue(text_lines(reference, candidate)['text_line_match'],
                                f'Logical {logical!r} must corroborate visual {visual!r}: {candidate}')

    def test_false_bidi_replacement_cannot_hide_behind_matching_poppler_output(self):
        visual = 'X תירבע'
        mapping = dict(enumerate(visual, start=65))
        paint = self.paint(''.join(chr(code) for code in mapping))
        reference = analyse(self.pdf(paint, unicode_map=mapping))['pages']
        for logical in [visual, 'X עברית 2', 'X עברתי']:
            with self.subTest(logical=logical):
                properties = serialize({'ActualText': b'\xfe\xff' + logical.encode('utf-16-be')})
                candidate = analyse(self.pdf(b'/Span ' + properties + b' BDC ' + paint + b'EMC',
                                             unicode_map=mapping))['pages']
                self.assertFalse(text_lines(reference, candidate)['text_line_match'],
                                 f'False logical text {logical!r} must fail even when extracted text agrees')
