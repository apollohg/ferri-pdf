import unittest
from unittest.mock import patch

import bidi_text


class BidiTextTests(unittest.TestCase):
    def tearDown(self):
        bidi_text.library.cache_clear()

    def test_missing_explicit_library_fails_without_falling_back(self):
        bidi_text.library.cache_clear()
        with patch.dict('os.environ', {'ORACLE_FRIBIDI_LIBRARY': '/nonexistent/oracle-fribidi'}):
            with self.assertRaisesRegex(bidi_text.BidiError, 'required to verify RTL'):
                bidi_text.visual_orders('עברית')

    def test_native_failure_is_not_an_empty_success(self):
        with patch('bidi_text.library') as library:
            library.return_value.fribidi_get_bidi_type.return_value = 0
            library.return_value.fribidi_log2vis.return_value = 0
            with self.assertRaisesRegex(bidi_text.BidiError, 'could not resolve'):
                bidi_text.visual_orders('עברית')

    def test_ordering_preserves_numbers_and_reports_native_version(self):
        self.assertIn('X דג 12 בא', bidi_text.visual_orders('X אב 12 גד'))
        self.assertNotIn('X דג 21 בא', bidi_text.visual_orders('X אב 12 גד'))
        self.assertIn('FriBidi', bidi_text.provenance()['version'])
        self.assertTrue(bidi_text.provenance()['library'])
