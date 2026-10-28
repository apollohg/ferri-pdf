from pathlib import Path
import tempfile
import unittest

from comparison import analyse
from conformance import check_assertions
from manifest import load_bundle
from test_support import valid_bundle, write_probe_pdf
from workloads import materialise_workloads, check_eligibility


class WorkloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    def test_materialisation_is_identical_and_suite_is_six_by_three(self):
        bundle = load_bundle(Path(__file__).with_name('manifests'))
        first=materialise_workloads(bundle,self.root/'first')
        second=materialise_workloads(bundle,self.root/'second')
        self.assertEqual(len(bundle['benchmarks']['classes']),6)
        self.assertEqual(len(first),18)
        for group in bundle['benchmarks']['classes']:
            self.assertEqual(len(group['case_ids']),3)
            self.assertEqual({bundle['cases'][i]['scale'] for i in group['case_ids']},{'small','medium','large'})
        for identity,path in first.items():
            self.assertEqual(path.read_bytes(),second[identity].read_bytes(),identity)
            self.assertNotIn(b'https://',path.read_bytes())
            self.assertNotIn(b'file:',path.read_bytes())
            for resource in bundle['cases'][identity]['resources']:
                self.assertEqual((path.parent/resource['path']).read_bytes(),(second[identity].parent/resource['path']).read_bytes())

    def test_large_unique_images_have_distinct_content(self):
        from hashlib import sha256
        from workloads import image_bytes
        self.assertEqual(len({sha256(image_bytes(i)).digest() for i in range(1,601)}),600)

    def test_multilingual_uses_actual_primary_and_fallback_fonts(self):
        from fontTools.ttLib import TTFont
        bundle=load_bundle(Path(__file__).with_name('manifests'));paths=materialise_workloads(bundle,self.root)
        case=bundle['cases']['bench-multilingual-small'];text=paths[case['id']].read_text()
        self.assertIn('STIXGeneral.ttf',text)
        primary=TTFont(Path(__file__).with_name('assets')/'STIXGeneral.ttf')
        fallback=TTFont(Path(__file__).with_name('assets')/'DejaVuSans.ttf')
        self.assertNotIn(ord('ש'),primary.getBestCmap());self.assertIn(ord('ש'),fallback.getBestCmap())
        expected=[a['expected'] for a in case['assertions'] if a['kind']=='glyphs']
        self.assertTrue(any(a.get('font')=='STIXGeneral-Regular' and a.get('codepoint_ranges') for a in expected))
        self.assertTrue(any(a.get('font')=='DejaVuSans' and a.get('codepoint_ranges') for a in expected))

    def test_unexpected_pdf_font_blocks_eligibility(self):
        bundle=valid_bundle(self.root/'manifest')
        case=bundle['cases']['invoices-small']
        case.update(resources=[],provenance={'input_sha256':'a'*64})
        case['assertions']=[{'id':'font','kind':'fonts','expected':{'names':['DejaVuSans'],'embedded':True},
            'basis':{'kind':'specification','source_id':'css-sizing-3','sections':['width'],'sha256':'a'*64,'reason':'Explicit probe font contract'}}]
        pdf=write_probe_pdf(self.root/'fallback.pdf')
        checks=check_assertions(case,{'candidate':pdf,'analysis':analyse(pdf),'status':'rendered'},bundle,self.root/'inspect')
        record={'status':'rendered','input_sha256':'a'*64,'resources_sha256':{},'assertions':checks}
        result=check_eligibility(case,record)
        self.assertEqual(result['status'],'blocked')
        self.assertIn('font',str(result['reasons']))

    def test_wrong_asset_dropped_rows_running_content_and_warnings_block(self):
        bundle=valid_bundle(self.root/'manifest');case=bundle['cases']['invoices-small']
        case['resources']=[{'path':'font.ttf','sha256':'a'*64}]
        case['provenance']['input_sha256']='b'*64
        case['assertions']=[{'id':identity,'kind':'unicode-text'} for identity in ('total','last-row','running-header')]
        valid={'status':'rendered','input_sha256':'b'*64,'resources_sha256':{'font.ttf':'a'*64},
               'assertions':[{'id':identity,'status':'pass'} for identity in ('total','last-row','running-header')]}
        import copy
        self.assertEqual(check_eligibility(case,valid)['status'],'eligible')
        for failure in ('asset','input','total','last-row','running-header','warning'):
            with self.subTest(failure=failure):
                bad=copy.deepcopy(valid)
                if failure=='asset':bad['resources_sha256']['font.ttf']='c'*64
                elif failure=='input':bad['input_sha256']='c'*64
                elif failure=='warning':bad['warnings_unresolved']=True
                else:next(a for a in bad['assertions'] if a['id']==failure)['status']='fail'
                result=check_eligibility(case,bad)
                self.assertEqual(result['status'],'blocked',result['reasons'])


if __name__=='__main__':unittest.main()
