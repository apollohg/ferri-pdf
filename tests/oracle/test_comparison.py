from pathlib import Path
import os
import shlex
import shutil
import tempfile
import unittest
from unittest.mock import patch

from test_support import write_probe_pdf


class ComparisonTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.reference = write_probe_pdf(self.root/'reference.pdf')

    def compare(self,variant):
        from comparison import compare_pdfs, default_settings
        candidate = write_probe_pdf(self.root/'candidate.pdf',variant)
        return compare_pdfs(self.reference,candidate,variant,output_dir=self.root/'analysis',settings=default_settings())

    def test_identical_real_pdf_passes_all_comparison_gates(self):
        result = self.compare('base')
        self.assertTrue(result['match'],result)
        self.assertTrue(result['glyph_match'])
        self.assertTrue(result['text_ink_match'])

    def test_missing_page_fails_actual_pdf_comparison(self):
        result = self.compare('missing-page')
        self.assertFalse(result['page_match'])
        self.assertFalse(result['match'])

    def test_missing_glyph_and_changed_colour_fail_glyph_checks(self):
        for variant in ('missing-glyph','colour'):
            with self.subTest(variant=variant):
                result = self.compare(variant)
                self.assertFalse(result['glyph_match'])
                self.assertFalse(result['match'])

    def test_subpoint_displacement_is_not_hidden_by_raster_tolerance(self):
        result = self.compare('subpoint-shift')
        self.assertFalse(result['glyph_match'])
        self.assertFalse(result['match'])

    def test_clipping_and_occlusion_cannot_pass_with_matching_glyph_records(self):
        for variant in ('clipped','occluded'):
            with self.subTest(variant=variant):
                result = self.compare(variant)
                self.assertTrue(result['glyph_match'])
                self.assertFalse(result['text_ink_match'])
                self.assertFalse(result['match'])

    def test_calibration_checks_real_geometry_and_colour(self):
        from comparison import calibrate,default_settings
        result = calibrate(self.root/'calibration',default_settings())
        self.assertTrue(result['two_point_glyph_shift_rejected'])
        self.assertTrue(result['missing_page_rejected'])
        self.assertTrue(result['colour_only_difference_rejected'])

    def test_brackets_in_pdf_names_cannot_skip_occlusion_checks(self):
        from comparison import compare_pdfs,default_settings
        reference = write_probe_pdf(self.root/'[reference].pdf')
        candidate = write_probe_pdf(self.root/'[candidate].pdf','occluded')
        result = compare_pdfs(reference,candidate,'brackets',output_dir=self.root/'brackets',settings=default_settings())
        self.assertFalse(result['text_ink_match'])
        self.assertFalse(result['match'])

    def test_rasterizer_success_without_page_images_is_not_visual_evidence(self):
        from comparison import AnalysisError
        directory = self.root/'bin'
        directory.mkdir()
        tool = directory/'pdftoppm'
        tool.write_text('#!/bin/sh\nexit 0\n')
        tool.chmod(0o755)
        with patch.dict(os.environ,{'PATH':str(directory)+os.pathsep+os.environ['PATH']}):
            with self.assertRaisesRegex(AnalysisError,'raster'):
                self.compare('base')

    def test_missing_second_raster_page_cannot_pass_a_two_page_pdf(self):
        from comparison import AnalysisError
        real = shutil.which('pdftoppm')
        directory = self.root/'bin'
        directory.mkdir()
        tool = directory/'pdftoppm'
        tool.write_text('#!/bin/sh\nexec '+shlex.quote(real)+' -f 1 -l 1 "$@"\n')
        tool.chmod(0o755)
        with patch.dict(os.environ,{'PATH':str(directory)+os.pathsep+os.environ['PATH']}):
            with self.assertRaisesRegex(AnalysisError,'raster'):
                self.compare('base')


if __name__ == '__main__':
    unittest.main()
