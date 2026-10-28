from pathlib import Path
import tempfile
import unittest

from test_support import TEST_REQUIREMENT_ID, valid_bundle, write_probe_pdf
from comparison import analyse, text_lines
from conformance import check_assertions, evaluate_requirements, evaluate_case


class ConformanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.bundle = valid_bundle(self.root/'manifest')
        self.case = self.bundle['cases']['invoices-small']
        self.requirement = self.bundle['requirements'][TEST_REQUIREMENT_ID]
        self.pdf = write_probe_pdf(self.root/'probe.pdf')
        self.artifacts = {'candidate': self.pdf, 'status': 'rendered', 'analysis': analyse(self.pdf)}

    def assertion(self,kind,expected,**extra):
        item = {'id': 'probe', 'kind': kind, 'expected': expected, 'basis': {
            'kind': 'specification', 'source_id': 'css-sizing-3', 'sections': ['width'],
            'sha256': 'a'*64, 'reason': 'Independently authored probe contract.'}, **extra}
        self.case['assertions'] = [item]
        self.requirement['review_state'] = 'reviewed'
        self.requirement['verification'] = [{'case_id': self.case['id'], 'assertion_id': item['id']}]
        return item

    def results(self):
        return check_assertions(self.case,self.artifacts,self.bundle,self.root/'checks')

    def test_case_status_requires_rendering_and_every_mandatory_assertion(self):
        self.assertion('page-count', 2)
        self.case['assertions'].append({**self.case['assertions'][0], 'id': 'optional', 'expected': 3, 'mandatory': False})
        record = {'status': 'rendered', 'assertions': self.results()}
        self.assertEqual([item['status'] for item in record['assertions']], ['pass', 'fail'])
        self.assertEqual(evaluate_case(self.case, record), 'pass', 'A failing optional assertion must not fail the case')
        self.assertEqual(evaluate_case(self.case, {**record, 'status': 'unsupported'}), 'blocked',
                         'Passing assertions cannot outlive a failed render')
        self.case['assertions'][1]['mandatory'] = True
        self.assertEqual(evaluate_case(self.case, record), 'fail')
        self.assertEqual(evaluate_case(self.case, {'status': 'rendered', 'assertions': []}), 'unverified',
                         'Missing assertion evidence must not pass')
        self.case['assertions'] = []
        self.assertEqual(evaluate_case(self.case, {'status': 'rendered', 'assertions': []}), 'unverified',
                         'A case without assertions has no independent evidence')

    def test_actual_pdf_geometry_paint_and_text_expectations(self):
        for kind,expected in [('page-count',2),('page-boxes',{'width_pt':150,'height_pt':100}),
                              ('unicode-text',{'text':'Probe','pages':[1,2],'count_per_page':1}),
                              ('glyphs',{'text':'ProbeProbe','font':'Helvetica','fill':[0,0,0,1],
                                         'origins_pt':[[36,36],[43.2,36],[50.4,36],[57.6,36],[64.8,36]]*2})]:
            with self.subTest(kind=kind):
                self.assertion(kind,expected)
                self.assertEqual(self.results()[0]['status'],'pass',self.results())
        self.assertion('glyphs',{'text':'Missing'})
        self.assertEqual(self.results()[0]['status'],'fail')

    def test_visual_word_inventory_handles_phrase_actualtext_without_trusting_its_claims(self):
        self.assertion('word-counts', {'minimum_counts': {'Alpha': 2, 'Beta': 2}, 'word_order': 'visual'})
        for payload, painted, expected in [
            ('Alpha Beta', 'Alpha Beta', 'pass'),
            ('Beta Alpha', 'Alpha Beta', 'pass'),
            ('Alpha Beta', 'Alpha', 'fail'),
            ('Alpha Beta', 'AlphaBeta', 'fail'),
            ('Alpha Beta', 'Alpha Zeta', 'fail'),
        ]:
            with self.subTest(payload=payload, painted=painted):
                content = (f'BT /F1 12 Tf 1 0 0 1 12 36 Tm '
                           f'/Span << /ActualText ({payload}) >> BDC ({painted}) Tj EMC ET').encode()
                pdf = write_probe_pdf(self.root/'phrase.pdf', content_stream=content)
                self.artifacts = {'candidate': pdf, 'status': 'rendered', 'analysis': analyse(pdf)}
                self.assertEqual(self.results()[0]['status'], expected, self.results())
                if expected == 'pass' and payload != painted:
                    reference = write_probe_pdf(self.root/'visual-reference.pdf', content_stream=
                        f'BT /F1 12 Tf 1 0 0 1 12 36 Tm ({painted}) Tj ET'.encode())
                    self.assertFalse(text_lines(analyse(reference)['pages'], self.artifacts['analysis']['pages'])['text_line_match'],
                                     'word counts must not waive replacement-text ordering differences')

    def test_visual_word_inventory_keeps_positioned_words_without_space_glyphs(self):
        self.assertion('word-counts', {'minimum_counts': {'Alpha': 2, 'Beta': 2}, 'word_order': 'visual'})
        content = b'BT /F1 12 Tf 1 0 0 1 12 36 Tm (Alpha) Tj 1 0 0 1 70 36 Tm (Beta) Tj ET'
        pdf = write_probe_pdf(self.root/'positioned-words.pdf', content_stream=content)
        self.artifacts = {'candidate': pdf, 'status': 'rendered', 'analysis': analyse(pdf)}
        self.assertEqual(self.results()[0]['status'], 'pass', self.results())

    def test_font_embedding_and_visible_region_use_real_pdf(self):
        self.assertion('fonts',{'names':['Helvetica'],'embedded':True})
        self.assertEqual(self.results()[0]['status'],'fail','Base14 probe font is not embedded')
        self.assertion('region-ink',{'page':1,'rectangle_pt':[30,50,90,70],'minimum_dark_pixels':10})
        self.assertEqual(self.results()[0]['status'],'pass')
        self.artifacts = {'candidate':write_probe_pdf(self.root/'clipped.pdf','clipped'),'status':'rendered'}
        self.assertEqual(self.results()[0]['status'],'fail','Clipped glyph records are insufficient painted-ink evidence')

    def test_missing_output_ambiguity_and_wrong_source_never_pass(self):
        item = self.assertion('unicode-text',{'text':'Probe','pages':'all'})
        self.assertEqual(self.results()[0]['status'],'pass')
        item['basis']['ambiguous'] = True
        self.assertEqual(self.results()[0]['status'],'unverified')
        item['basis'].pop('ambiguous')
        item['basis']['sha256'] = 'b'*64
        self.assertEqual(self.results()[0]['status'],'unverified')
        self.artifacts['candidate'] = self.root/'absent.pdf'
        self.assertEqual(self.results()[0]['status'],'blocked')

    def test_failure_dominates_blocked_dependencies_and_exclusions_do_not_pass(self):
        self.assertion('page-count',3)
        dependency = dict(self.requirement,id='sizing.dependency',verification=[],dependencies=[])
        self.bundle['requirements'][dependency['id']] = dependency
        self.requirement['dependencies'] = [dependency['id']]
        records = {self.case['id']:{'status':'rendered','assertions':self.results()}}
        states = evaluate_requirements(self.bundle,records)
        self.assertEqual(states[TEST_REQUIREMENT_ID]['status'],'fail')
        self.assertEqual(states[dependency['id']]['status'],'unverified')
        self.requirement['applicability']['status'] = 'excluded'
        self.assertEqual(evaluate_requirements(self.bundle,records)[TEST_REQUIREMENT_ID]['status'],'not_applicable')

    def test_any_page_pattern_and_per_page_contracts(self):
        self.assertion('unicode-text',{'texts':['Probe'],'pages':'any'})
        self.assertEqual(self.results()[0]['status'],'pass')
        self.case['assertions'][0]['expected']={'patterns':['Pr.*be'],'pages':'any'}
        self.assertEqual(self.results()[0]['status'],'pass')
        self.case['assertions'][0]['expected']={'patterns':['Pr.*missing'],'pages':'any'}
        self.assertEqual(self.results()[0]['status'],'fail')
        self.case['assertions'][0]['expected']={'per_page':[{'text':'Probe','pages':[1],'count_per_page':1},{'text':'Probe','pages':[2],'count_per_page':2}]}
        self.assertEqual(self.results()[0]['status'],'fail')

    def test_failed_execution_invalidates_previous_passing_assertion_evidence(self):
        self.assertion('page-count',2)
        record={'status':'unsupported','assertions':self.results()}
        self.assertEqual(record['assertions'][0]['status'],'pass')
        self.assertEqual(evaluate_requirements(self.bundle,{self.case['id']:record})[TEST_REQUIREMENT_ID]['status'],'blocked')

    def test_missing_assertion_and_unknown_kind_are_unverified(self):
        self.assertion('future-inspection',{})
        self.assertEqual(self.results()[0]['status'],'unverified')
        records={self.case['id']:{'status':'rendered','assertions':[]}}
        self.assertEqual(evaluate_requirements(self.bundle,records)[TEST_REQUIREMENT_ID]['status'],'unverified')


if __name__ == '__main__':
    unittest.main()
