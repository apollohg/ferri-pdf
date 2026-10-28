from pathlib import Path
import tempfile
import unittest

from pdf_content import Name, Ref, PdfError, navigation, serialize
from test_support import write_pdf_objects


class NamedDestinationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.path = Path(temporary.name) / 'navigation.pdf'

    def objects(self, target):
        first = [Ref(3, 0), Name('XYZ'), 0, 90, None]
        second = [Ref(4, 0), Name('XYZ'), 0, 80, None]
        return [
            {'Type': Name('Catalog'), 'Pages': Ref(2, 0),
             'Names': {'Dests': Ref(6, 0)}, 'Dests': {'launch': first},
             'Outlines': Ref(10, 0)},
            {'Type': Name('Pages'), 'Kids': [Ref(3, 0), Ref(4, 0)], 'Count': 2},
            {'Type': Name('Page'), 'Parent': Ref(2, 0), 'MediaBox': [0, 0, 100, 100],
             'Annots': [Ref(5, 0)]},
            {'Type': Name('Page'), 'Parent': Ref(2, 0), 'MediaBox': [0, 0, 100, 100]},
            {'Subtype': Name('Link'), 'Rect': [0, 0, 20, 20], 'Dest': target},
            {'Kids': [Ref(7, 0), Ref(8, 0)]},
            {'Names': [b'alpha', first], 'Limits': [b'alpha', b'alpha']},
            {'Names': [b'launch', Ref(9, 0)], 'Limits': [b'launch', b'launch']},
            {'D': second},
            {'Type': Name('Outlines'), 'First': Ref(11, 0), 'Last': Ref(11, 0), 'Count': 1},
            {'Title': b'Launch', 'Parent': Ref(10, 0), 'Dest': target},
        ]

    def read(self, objects):
        write_pdf_objects(self.path, [serialize(value) for value in objects])
        return navigation(self.path)

    def test_name_and_byte_string_destinations_use_their_declared_namespaces(self):
        for target, expected in [(Name('launch'), {'page': 1, 'top': 90}),
                                 (b'launch', {'page': 2, 'top': 80})]:
            with self.subTest(target=target):
                result = self.read(self.objects(target))
                self.assertEqual(result['links'][0]['destination'], expected)
                self.assertEqual(result['outline'][0]['destination'], expected)

    def test_goto_action_and_binary_name_keys_resolve_without_text_decoding(self):
        key = b'\xff\x00target'
        objects = self.objects(key)
        objects[4].pop('Dest')
        objects[4]['A'] = {'S': Name('GoTo'), 'D': key}
        objects[7] = {'Names': [key, Ref(9, 0)], 'Limits': [key, key]}
        result = self.read(objects)
        self.assertEqual(result['links'][0]['destination'], {'page': 2, 'top': 80})
        self.assertEqual(result['outline'][0]['destination'], {'page': 2, 'top': 80})

    def test_missing_name_and_malformed_name_tree_are_explicit_failures(self):
        with self.assertRaisesRegex(PdfError, 'missing named destination'):
            self.read(self.objects(b'missing'))
        for malformed in [{'Kids': [Ref(6, 0)]}, {'Names': [b'launch']},
                          {'Names': [b'launch', Ref(9, 0), b'launch', Ref(9, 0)]}]:
            with self.subTest(tree=malformed), self.assertRaises(PdfError):
                objects = self.objects(b'launch')
                objects[5] = malformed
                self.read(objects)

    def test_comparison_reports_invalid_navigation_as_an_analysis_failure(self):
        from comparison import AnalysisError, compare_pdfs, default_settings
        objects = self.objects(b'missing')
        write_pdf_objects(self.path, [serialize(value) for value in objects])
        with self.assertRaisesRegex(AnalysisError, 'navigation.*missing named destination'):
            compare_pdfs(self.path, self.path, 'missing-destination',
                         settings=default_settings(), output_dir=self.path.parent / 'analysis')

    def test_cyclic_indirect_destination_value_fails_explicitly(self):
        objects = self.objects(b'launch')
        objects[8] = Ref(9, 0)
        with self.assertRaisesRegex(PdfError, 'cyclic object reference'):
            self.read(objects)
