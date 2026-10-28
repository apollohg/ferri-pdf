import copy
from pathlib import Path
import tempfile
import unittest

from test_support import TEST_REQUIREMENT_ID, TEST_SOURCE_ID, valid_bundle


class InventoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.bundle = valid_bundle(self.root)

    def test_computed_property_without_rendering_evidence_is_unverified(self):
        from manifest import inventory_summary
        self.bundle['requirements'][TEST_REQUIREMENT_ID]['implementation']['status'] = 'present'
        summary = inventory_summary(self.bundle)
        self.assertIn(TEST_REQUIREMENT_ID, summary['missing_evidence_ids'])
        self.assertEqual(summary['verified'], 0)

    def test_exclusions_do_not_increase_the_passing_count(self):
        from manifest import inventory_summary
        requirement = self.bundle['requirements'][TEST_REQUIREMENT_ID]
        requirement['applicability'] = {'status': 'excluded', 'reason': 'Only live scripting is specified here.', 'basis': {'source_id': TEST_SOURCE_ID, 'section_id': 'width', 'classification': 'interactive-only'}}
        summary = inventory_summary(self.bundle)
        self.assertEqual(summary['excluded'], 1)
        self.assertEqual(summary['applicable'], 0)
        self.assertEqual(summary['verified'], 0)

    def test_superseded_section_needs_an_existing_controlling_section(self):
        from manifest import ManifestError, validate_bundle
        self.bundle['sections'][TEST_SOURCE_ID].append({'id': 'old-width', 'title': 'Old Width', 'disposition': {'kind': 'superseded', 'reason': 'Later requirements control.', 'controlling': {'source_id': TEST_SOURCE_ID, 'section_id': 'missing'}}})
        with self.assertRaisesRegex(ManifestError, 'controlling'):
            validate_bundle(self.bundle)

    def test_section_cannot_be_disposed_by_an_unrelated_requirement(self):
        from manifest import ManifestError, validate_bundle
        self.bundle['sections'][TEST_SOURCE_ID].append({'id': 'fragmentation', 'title': 'Fragmentation', 'disposition': {'kind': 'requirements', 'requirement_ids': [TEST_REQUIREMENT_ID]}})
        with self.assertRaisesRegex(ManifestError, 'section'):
            validate_bundle(self.bundle)

    def test_unresolved_imports_prevent_complete_source_closure(self):
        from manifest import inventory_summary
        self.bundle['sources'][TEST_SOURCE_ID]['reference_dispositions'] = [{'url': 'https://example.test/unpinned', 'status': 'unresolved', 'section_id': 'width'}]
        summary = inventory_summary(self.bundle)
        self.assertFalse(summary['source_closure_complete'])
        self.assertEqual(len(summary['unresolved_imports']), 1)

    def test_published_font_definitions_are_never_excluded_by_heading_spelling(self):
        from manifest import load_bundle
        bundle=load_bundle(Path(__file__).with_name('manifests'),scope='inventory')
        for source,identity in [('opentype-head','head--font-header-table'),('opentype-cff2','index-data'),('opentype-cff2','head-table')]:
            with self.subTest(source=source,section=identity):
                section=next(item for item in bundle['sections'][source] if item['id']==identity)
                self.assertEqual(section['disposition']['kind'],'requirements')
                self.assertTrue(section['disposition']['requirement_ids'])
        for sections in bundle['sections'].values():
            for section in sections:
                self.assertNotEqual(section.get('disposition',{}).get('reason'),'Publication metadata, bibliography, indexes or change history; operative definitions are mapped at their normative sections.')

    def test_overlapping_publications_record_controllers_and_unresolved_scope(self):
        from manifest import load_bundle,inventory_summary
        bundle=load_bundle(Path(__file__).with_name('manifests'),scope='inventory')
        old=next(item for item in bundle['sections']['css21-cascade'] if item['id']=='cascading-order')
        self.assertEqual(old['disposition']['kind'],'superseded')
        self.assertEqual(old['disposition']['controlling'],{'source_id':'css-cascade-5','section_id':'cascade-sort'})
        sizing=next(item for item in bundle['sections']['css21-visudet'] if item['id']=='min-max-widths')
        override=next(item for item in sizing['controlling_definitions'] if item['behaviour']=='min-width initial value')
        self.assertEqual(override['controlling'],{'source_id':'css-sizing-3','section_id':'min-size-properties'})
        summary=inventory_summary(bundle)
        self.assertTrue(summary['unresolved_overlaps'],'Unreviewed cross-level overlap must remain visible')
        self.assertFalse(summary['inventory_complete'])

    def test_controlling_definitions_and_overlap_records_need_pinned_targets(self):
        from manifest import validate_bundle,ManifestError
        self.bundle['sections'][TEST_SOURCE_ID][0]['controlling_definitions']=[{'behaviour':'initial value','controlling':{'source_id':'missing','section_id':'width'},'basis':{'source_id':TEST_SOURCE_ID,'section_id':'width','sha256':'a'*64}}]
        with self.assertRaises(ManifestError):validate_bundle(self.bundle)
        self.bundle['sections'][TEST_SOURCE_ID][0].pop('controlling_definitions')
        self.bundle['sources'][TEST_SOURCE_ID]['overlap_dispositions']=[{'status':'unresolved','other_source_id':'missing','reason':'Review required'}]
        with self.assertRaises(ManifestError):validate_bundle(self.bundle)

    def test_static_form_and_nonproperty_behaviours_are_counted(self):
        from manifest import inventory_summary, validate_bundle
        requirements = self.bundle['requirements']
        for identity, behaviour in [('html.static-form', 'Render a disabled form control in its initial state.'), ('fragmentation.forced-break', 'A forced break advances to the required page.')]:
            record = copy.deepcopy(requirements[TEST_REQUIREMENT_ID])
            record.update(id=identity, behaviour=behaviour)
            requirements[identity] = record
            self.bundle['sections'][TEST_SOURCE_ID][0]['disposition']['requirement_ids'].append(identity)
        validate_bundle(self.bundle)
        summary = inventory_summary(self.bundle)
        self.assertEqual(summary['applicable'], 3)
        self.assertIn('html.static-form', summary['missing_evidence_ids'])
        self.assertIn('fragmentation.forced-break', summary['missing_evidence_ids'])


if __name__ == '__main__':
    unittest.main()
