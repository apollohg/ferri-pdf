import copy
from pathlib import Path
import tempfile
import unittest

from test_support import valid_bundle
from benchmark_stats import aggregate


class BenchmarkStatisticsTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        bundle=valid_bundle(Path(self.temp.name))
        self.suite=bundle['benchmarks'];self.ids=list(bundle['cases'])
        self.eligibility={identity:{'status':'eligible'} for identity in self.ids}
        self.samples={identity:[{'run_index':i,'elapsed_ms':5,'status':'valid'} for i in range(30)] for identity in self.ids}

    def summarise(self):return aggregate(self.samples,self.suite,self.eligibility)

    def test_complete_suite_reports_each_document_median_and_range(self):
        for index,run in enumerate(self.samples[self.ids[0]]):run['elapsed_ms']=[10,2,7][index%3]
        result=self.summarise()
        self.assertEqual(result['status'],'valid',result['reasons'])
        self.assertEqual(len(result['documents']),18)
        document=result['documents'][self.ids[0]]
        self.assertEqual((document['median_ms'],document['minimum_ms'],document['maximum_ms']),(7,2,10))
        self.assertEqual(result['documents'][self.ids[1]]['median_ms'],5)

    def test_missing_suite_case_blocks_without_statistics(self):
        missing=self.ids[-1];self.samples.pop(missing)
        result=self.summarise()
        self.assertEqual(result['status'],'blocked');self.assertIn(missing,result['missing_case_ids'])
        self.assertIsNone(result['documents'][missing]['median_ms'])
        self.assertEqual(result['documents'][self.ids[0]]['status'],'valid')

    def test_failures_bad_durations_and_incomplete_runs_block_without_dropping(self):
        for invalid in [0,-1,float('nan'),float('inf'),True,None]:
            with self.subTest(duration=invalid):
                data=copy.deepcopy(self.samples);data[self.ids[0]][-1]['elapsed_ms']=invalid
                result=aggregate(data,self.suite,self.eligibility)
                self.assertEqual(result['status'],'blocked');self.assertIsNone(result['documents'][self.ids[0]]['median_ms'])
                self.assertEqual(result['documents'][self.ids[0]]['observations'],30)
        self.samples[self.ids[0]][-1]['status']='invalid'
        self.assertEqual(self.summarise()['status'],'blocked')
        self.samples[self.ids[0]][-1]['status']='valid'
        self.samples[self.ids[0]].pop()
        self.assertEqual(self.summarise()['status'],'blocked')

    def test_duplicate_runs_and_ineligible_output_block(self):
        data=copy.deepcopy(self.samples);data[self.ids[0]][-1]['run_index']=0
        self.assertEqual(aggregate(data,self.suite,self.eligibility)['status'],'blocked')
        self.eligibility[self.ids[0]]={'status':'blocked','reasons':['Missing table rows']}
        self.assertEqual(self.summarise()['status'],'blocked')


if __name__=='__main__':unittest.main()
