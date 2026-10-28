import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from test_support import write_probe_pdf
from benchmark import benchmark_case
from execution import rss_bytes

WARMUP_RUNS = 3
TIMED_RUNS = 30
RSS_RUNS = 1
ALL_RUNS = WARMUP_RUNS + TIMED_RUNS + RSS_RUNS


class BenchmarkTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name).resolve()
        self.input=self.root/'input.html';self.input.write_text('<p>Probe</p>')
        self.reference=write_probe_pdf(self.root/'probe.pdf')
        self.case={'id':'probe','timeout_s':2,'fingerprint_paths':[str(self.input)]}
        self.protocol={'warmup_runs':WARMUP_RUNS,'timed_runs':TIMED_RUNS}
        script='from pathlib import Path;import sys;Path(sys.argv[2]).write_bytes(Path(sys.argv[1]).read_bytes())'
        self.command=[sys.executable,'-c',script,str(self.reference),'{output}']
        self.checks=[]

    def verify(self,path):
        self.assertTrue(path.is_file(),f'missing attempt output {path}')
        self.checks.append(path)
        return {'status':'pass','pdf_bytes':path.stat().st_size}

    def run_case(self):return benchmark_case(self.case,self.command,self.protocol,self.root/'run',self.verify)

    def test_resource_change_invalidates_completed_timings(self):
        original=self.verify
        def verify(path):
            result=original(path)
            if len(self.checks)==WARMUP_RUNS+TIMED_RUNS:self.input.write_text('Changed after final run')
            return result
        result=benchmark_case(self.case,self.command,self.protocol,self.root/'run',verify)
        self.assertEqual(result['status'],'blocked');self.assertEqual(len(result['runs']),TIMED_RUNS)
        self.assertNotEqual(result['fingerprints_before'],result['fingerprints_after'])

    def test_changed_input_before_measurement_is_rejected_against_preflight(self):
        from execution import digest
        self.case['expected_fingerprints']={str(path):digest(path) for path in (self.input,Path(sys.executable))}
        self.input.write_text('Changed after preflight, before timing')
        result=self.run_case()
        self.assertEqual(result['status'],'blocked',result['reasons'])
        self.assertFalse(result['runs'],'Preflight identity mismatch must not obtain scored observations')
        self.assertIn('preflight',str(result['reasons']).lower())

    def test_exact_runs_and_each_real_pdf_is_verified(self):
        result=self.run_case()
        self.assertEqual(result['status'],'valid',result['reasons'])
        self.assertEqual(len(result['warmups']),WARMUP_RUNS);self.assertEqual(len(result['runs']),TIMED_RUNS)
        self.assertEqual(len(self.checks),ALL_RUNS,'Every warmup, timed and separate RSS output is verified')
        self.assertEqual(len(set(self.checks)),ALL_RUNS,'Every observation owns a fresh PDF')
        for stage in ('warmups','runs'):
            for index,run in enumerate(result[stage]):
                self.assertEqual(run['run_index'],index)
                self.assertNotIn('--trace',str(run))
                self.assertEqual(run['attempt']['status'],'rendered')
                self.assertGreater(run['elapsed_ms'],0)
        self.assertEqual(json.loads((self.root/'run'/'benchmark-case.json').read_text())['status'],'valid')

    def test_timeout_attempt_is_retained(self):
        self.protocol={'warmup_runs':0,'timed_runs':1};self.case['timeout_s']=.05
        command=[sys.executable,'-c','import time;time.sleep(5)','{output}']
        result=benchmark_case(self.case,command,self.protocol,self.root/'timeout',lambda path:{'status':'pass'})
        self.assertEqual(result['status'],'blocked');self.assertEqual(len(result['runs']),1)
        self.assertEqual(result['runs'][0]['attempt']['status'],'timeout')

    def test_invalid_missing_and_stale_outputs_are_retained(self):
        self.protocol={'warmup_runs':0,'timed_runs':1}
        for name,script in [('invalid','import sys;open(sys.argv[1],"w").write("broken")'),('missing','pass'),('stale','import os,sys;from pathlib import Path;p=Path(sys.argv[1]);p.write_bytes(b"%PDF-1.4\\n%%EOF\\n");os.utime(p,(1,1))')]:
            with self.subTest(outcome=name):
                command=[sys.executable,'-c',script,'{output}']
                result=benchmark_case(self.case,command,self.protocol,self.root/name,lambda path:{'status':'pass'})
                self.assertEqual(result['status'],'blocked');self.assertEqual(len(result['runs']),1)
                observation=result['runs'][0]['attempt']
                self.assertEqual(observation['execution']['status'],'completed',observation)
                self.assertEqual(observation['execution']['exit_status'],0,observation)
                self.assertEqual(observation['status'],'render_error',observation)

    def test_ineligible_suite_members_are_reported_without_timings(self):
        from benchmark import benchmark_suite
        from test_support import valid_bundle
        bundle=valid_bundle(self.root/'manifests')
        result=benchmark_suite(bundle,{},self.root/'suite')
        self.assertTrue(result['complete'])
        self.assertEqual(set(result['cases']),set(bundle['cases']))
        self.assertTrue(all(record['status']=='blocked' and not record['runs'] for record in result['cases'].values()))
        self.assertEqual(len(result['aggregate']['documents']),18)
        self.assertEqual(result['aggregate']['status'],'blocked')

    def test_render_warning_cannot_pass_because_pdf_exists(self):
        self.protocol={'warmup_runs':0,'timed_runs':1}
        self.command[2]+=';print("warning: image resource could not be decoded",file=sys.stderr)'
        result=self.run_case()
        self.assertEqual(result['status'],'blocked')
        self.assertEqual(len(result['runs']),1)
        self.assertTrue(result['runs'][0]['attempt']['warnings_unresolved'])

    def test_interrupt_retains_partial_run_without_replacement(self):
        def interrupt(path):raise KeyboardInterrupt
        with self.assertRaises(KeyboardInterrupt):
            benchmark_case(self.case,self.command,self.protocol,self.root/'interrupt',interrupt)
        saved=json.loads((self.root/'interrupt'/'benchmark-case.json').read_text())
        self.assertTrue(saved['interrupted']);self.assertEqual(saved['status'],'blocked')
        self.assertEqual(len(saved['warmups']),1);self.assertEqual(len(saved['runs']),0)
        self.assertIn('execution',saved['warmups'][0]['attempt'])

    def test_rss_only_corrupt_output_blocks_case(self):
        from execution import digest
        def runner(argv,directory,timeout_s,**kwargs):
            path=Path(argv[-1])
            if kwargs.get('collect_rss'):write_probe_pdf(path,'missing-glyph')
            else:path.write_bytes(self.reference.read_bytes())
            return {'argv':argv,'status':'completed','exit_status':0,'elapsed_ms':1,'started_ns':0,'stdout_path':None,'stderr_path':None,'peak_rss_bytes':1234}
        def verify(path):return {'status':'pass' if digest(path)==digest(self.reference) else 'blocked'}
        with patch('benchmark.run_command',side_effect=runner):
            result=benchmark_case(self.case,self.command,self.protocol,self.root/'rss',verify)
        self.assertEqual(result['status'],'blocked','A corrupt untimed RSS output invalidates the case')
        self.assertEqual(len(result['runs']),TIMED_RUNS,'Completed timed runs must remain available')
        self.assertTrue(all(run['status']=='valid' for run in result['runs']))
        self.assertEqual(result['rss']['verification']['status'],'blocked')
        self.assertEqual(result['rss']['peak_rss_bytes'],1234)

    def test_interrupted_rss_keeps_active_execution(self):
        original=self.verify
        def verify(path):
            value=original(path)
            if len(self.checks)==ALL_RUNS:raise KeyboardInterrupt
            return value
        with self.assertRaises(KeyboardInterrupt):
            benchmark_case(self.case,self.command,self.protocol,self.root/'rss-interrupt',verify)
        saved=json.loads((self.root/'rss-interrupt/benchmark-case.json').read_text())
        self.assertIsNotNone(saved['rss'],'Interrupted RSS must retain its active record')
        self.assertIn('execution',saved['rss'])
        self.assertTrue(saved['interrupted'])
        self.assertEqual(len(saved['runs']),TIMED_RUNS)

    def test_slow_last_observation_is_not_trimmed_and_analysis_is_untimed(self):
        executions=[]
        def runner(argv,output_dir,timeout_s,**kwargs):
            path=Path(argv[-1]);path.write_bytes(self.reference.read_bytes())
            index=len(executions);executions.append(argv)
            return {'argv':argv,'status':'completed','exit_status':0,'elapsed_ms':1000 if index==WARMUP_RUNS+TIMED_RUNS-1 else 1,
                    'started_ns':0,'stdout_path':None,'stderr_path':None,'peak_rss_bytes':None}
        with patch('benchmark.run_command',side_effect=runner):
            result=self.run_case()
        self.assertEqual(len(executions),ALL_RUNS)
        self.assertEqual(result['runs'][-1]['elapsed_ms'],1000)
        self.assertEqual(result['runs'][0]['elapsed_ms'],1)
        self.assertEqual(rss_bytes(12,'Darwin'),12);self.assertEqual(rss_bytes(12,'Linux'),12*1024)
        self.assertIsNone(rss_bytes(None,'Other'))


if __name__=='__main__':unittest.main()
