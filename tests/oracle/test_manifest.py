import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from test_support import TEST_REQUIREMENT_ID, TEST_SOURCE_ID, valid_bundle, write_json


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.bundle = valid_bundle(self.root)

    def api(self):
        import manifest
        return manifest

    def test_duplicate_requirement_ids_are_rejected(self):
        api = self.api()
        requirement_file = self.root / "requirements" / "sizing.json"
        value = json.loads(requirement_file.read_text())
        value["requirements"].append(copy.deepcopy(value["requirements"][0]))
        write_json(requirement_file, value)
        with self.assertRaises(api.ManifestError) as caught:
            api.load_bundle(self.root)
        self.assertIn("duplicate", caught.exception.reason)
        self.assertEqual(caught.exception.file, requirement_file)
        self.assertIn("requirements", caught.exception.pointer)

    def test_valid_profile_loads_without_any_rendering_tools(self):
        api = self.api()
        bundle = api.load_bundle(self.root)
        self.assertEqual(len(bundle["cases"]), 18)
        self.assertEqual(api.select_cases(bundle, ["invoices-small"])[0]["id"], "invoices-small")
        script = Path(__file__).with_name("run.py")
        completed = subprocess.run([sys.executable, str(script), "--manifest-dir", str(self.root), "--validate-manifests"], capture_output=True, text=True)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("valid", completed.stdout.lower())

    def test_public_profile_inputs_are_bundled_and_locked(self):
        from execution import digest
        bundle=self.api().load_bundle(Path(__file__).with_name('manifests'))
        repository=Path(__file__).resolve().parents[2]
        bundled_fixtures=(repository/'tests/oracle/fixtures').resolve()
        for identity,case in bundle['cases'].items():
            if case['collection']=='benchmark':
                continue
            with self.subTest(case=identity):
                self.assertEqual(case['collection'],'synthetic',
                                 'Public profile must not depend on an external collection')
                self.assertTrue(case['assertions'],f'{identity}: a case needs independent output assertions')
                source=(repository/case['input']).resolve()
                self.assertTrue(source.is_relative_to(bundled_fixtures),
                                f'{identity}: input must be bundled beneath {bundled_fixtures}')
                self.assertTrue(source.is_file(),f'{identity}: bundled input is missing')
                self.assertEqual(digest(source),case['provenance']['input_sha256'],
                                 f'{identity}: committed input differs from its lock')

    def test_shared_dependency_is_valid_but_cycle_is_not(self):
        api = self.api()
        other = copy.deepcopy(self.bundle["requirements"][TEST_REQUIREMENT_ID])
        other["id"] = "sizing.minimum"
        other["dependencies"] = [TEST_REQUIREMENT_ID]
        self.bundle["requirements"][other["id"]] = other
        api.validate_bundle(self.bundle)
        self.bundle["requirements"][TEST_REQUIREMENT_ID]["dependencies"] = [other["id"]]
        with self.assertRaisesRegex(api.ManifestError, "cycle"):
            api.validate_bundle(self.bundle)

    def test_dangling_references_and_unknown_case_selection_are_rejected(self):
        api = self.api()
        mutations = (
            ("source", lambda b: b["requirements"][TEST_REQUIREMENT_ID].update(source_id="missing")),
            ("requirement", lambda b: b["cases"]["invoices-small"].update(requirement_ids=["missing"])),
            ("case", lambda b: b["requirements"][TEST_REQUIREMENT_ID].update(case_ids=["missing"])),
        )
        for kind, mutate in mutations:
            with self.subTest(reference=kind):
                bundle = copy.deepcopy(self.bundle)
                mutate(bundle)
                with self.assertRaises(api.ManifestError):
                    api.validate_bundle(bundle)
        with self.assertRaisesRegex(api.ManifestError, "unknown case"):
            api.select_cases(self.bundle, ["missing"])

    def test_missing_pin_bad_timeout_and_unmapped_section_are_rejected(self):
        api = self.api()
        mutations = (
            lambda b: b["sources"][TEST_SOURCE_ID].pop("sha256"),
            lambda b: b["sources"][TEST_SOURCE_ID].update(url="https://www.w3.org/TR/css-sizing-3/"),
            lambda b: b["cases"]["invoices-small"].update(timeout_s=0),
            lambda b: b["sections"][TEST_SOURCE_ID].append({"id": "height", "title": "Height"}),
        )
        for index, mutate in enumerate(mutations):
            with self.subTest(mutation=index):
                bundle = copy.deepcopy(self.bundle)
                mutate(bundle)
                with self.assertRaises(api.ManifestError):
                    api.validate_bundle(bundle)

    def test_malformed_nested_contracts_raise_actionable_manifest_errors(self):
        api = self.api()
        mutations = [
            lambda b:b['sources'][TEST_SOURCE_ID].update(url=[]),
            lambda b:b['requirements'][TEST_REQUIREMENT_ID].update(applicability=[]),
            lambda b:b['requirements'][TEST_REQUIREMENT_ID].update(implementation=[]),
            lambda b:b['cases']['invoices-small'].update(provenance=[]),
            lambda b:b['cases']['invoices-small'].update(resources=[{'path':'../escape','sha256':'a'*64}]),
            lambda b:b['cases']['invoices-small'].update(assertions=[{'id':'bad','kind':'page-count','expected':True}]),
            lambda b:b['cases']['invoices-small'].update(assertions=[{'id':'bad','kind':'region-ink','expected':{'page':1,'rectangle_pt':[0,0,10]}}]),
            lambda b:b['requirements'][TEST_REQUIREMENT_ID].update(verification=[{'case_id':'invoices-small','assertion_id':'missing'}]),
            lambda b:b['benchmarks'].update(classes=[None]),
            lambda b:b['requirements'][TEST_REQUIREMENT_ID].update(verification=None),
            lambda b:b['requirements'][TEST_REQUIREMENT_ID]['applicability'].update(status=[]),
            lambda b:b['cases']['invoices-small'].update(assertions=[{'id':[],'kind':'future','expected':{}}]),
            lambda b:b['benchmarks']['classes'][0].update(id=[]),
            lambda b:b['sections'][TEST_SOURCE_ID][0].update(disposition={'kind':[]}),
            lambda b:b['defaults'].update(tolerances=[]),
        ]
        for index,mutate in enumerate(mutations):
            with self.subTest(mutation=index):
                bundle=copy.deepcopy(self.bundle);mutate(bundle)
                with self.assertRaises(api.ManifestError) as caught:api.validate_bundle(bundle)
                self.assertTrue(caught.exception.pointer)

    def test_nonfinite_incomplete_and_malformed_tolerances_are_rejected(self):
        api=self.api()
        for field,value in [('glyph_origin_pt',float('inf')),('glyph_origin_pt',float('nan')),('resolution_dpi',0),('changed_ink_fraction',2),('word_box_em',[]),('metric_compatible_fonts',{'A':[]}),('unknown_field',1)]:
            with self.subTest(field=field,value=value):
                bundle=copy.deepcopy(self.bundle)
                bundle['defaults']['tolerances'][field]=value
                with self.assertRaises(api.ManifestError) as raised:api.validate_bundle(bundle)
                self.assertIn('/tolerances',raised.exception.pointer)
        bundle=copy.deepcopy(self.bundle);bundle['defaults']['tolerances'].pop('glyph_origin_pt')
        with self.assertRaises(api.ManifestError):api.validate_bundle(bundle)
        path=self.root/'defaults.json'
        original=path.read_text()
        for literal in ('Infinity','NaN','1e309'):
            path.write_text(original.replace('0.25',literal))
            with self.assertRaises(api.ManifestError):api.load_bundle(self.root)

    def test_inventory_scope_does_not_require_future_suite_or_cases(self):
        api = self.api()
        for name in ("cases.json", "defaults.json", "benchmarks.json"):
            (self.root / name).unlink()
        bundle = api.load_bundle(self.root, scope="inventory")
        self.assertEqual(bundle["scope"], "inventory")
        self.assertEqual(bundle["cases"], {})
        with self.assertRaises(api.ManifestError):
            api.load_bundle(self.root)


if __name__ == "__main__":
    unittest.main()
