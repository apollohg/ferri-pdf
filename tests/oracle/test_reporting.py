import json
from pathlib import Path
import re
import tempfile
import subprocess
import sys
import unittest
from urllib.parse import unquote

from comparison import REQUIRED_CALIBRATION_CHECKS
from execution import write_report
from reporting import calibration_fingerprint, build_summary, gate_statuses, profile_fingerprint, write_markdown
from test_support import TEST_REQUIREMENT_ID, valid_bundle


class ReportingTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name).resolve();self.bundle=valid_bundle(self.root/'manifest')
        self.ids=list(self.bundle['cases'])
        self.assertion={'id':'width','kind':'page-boxes','expected':{'width_pt':100,'height_pt':100},'basis':{'kind':'specification','source_id':'css-sizing-3','sections':['width'],'sha256':'a'*64,'reason':'Explicit page fixture'}}
        for case in self.bundle['cases'].values():case['assertions']=[dict(self.assertion)]
        req=self.bundle['requirements'][TEST_REQUIREMENT_ID]
        req.update(review_state='reviewed',verification=[{'case_id':self.ids[0],'assertion_id':'width'}])
        self.report={'schema_version':1,'complete':True,'selected_case_ids':self.ids,'cases':{i:{'case_id':i,'status':'rendered','assertions':[{'id':'width','status':'pass','actual':100,'expected':100,'evidence':{},'diagnostics':[]}]} for i in self.ids},'infrastructure_errors':[], 'optional_tools':{'weasyprint':'not_requested'},'profile_sha256':profile_fingerprint(self.bundle),'provenance':{'source_commit':'a'*40,'files':{'input':'b'*64}},'comparison_calibration':{'settings_sha256':calibration_fingerprint(self.bundle['defaults']['tolerances']),'checks':dict.fromkeys(REQUIRED_CALIBRATION_CHECKS,True)}}

    def test_cli_full_profile_enforces_global_source_closure(self):
        from types import SimpleNamespace
        from run import enforce_gates, EXIT_CONFORMANCE_FAILURE
        args=SimpleNamespace(require_conformance=True)
        self.report['gates']={'selected_conformance':'pass','conformance':'blocked'}
        self.report['scope']='full-profile'
        with self.assertRaises(SystemExit) as raised:enforce_gates(args,self.report)
        self.assertEqual(raised.exception.code,EXIT_CONFORMANCE_FAILURE)
        self.report['scope']='diagnostic'
        enforce_gates(args,self.report)

    def test_suite_publication_preserves_preflight_assertion_outcomes(self):
        from run import sync_suite_evidence
        identity=self.ids[0]
        record={'status':'rendered','assertions':[{'id':'width','status':'pass'}],'pdf':str(self.root/'candidate.pdf')}
        target=self.root/'preflight.json'
        write_report(target,{'cases':{identity:{'status':'eligible','record':record}}})
        report={'cases':{},'preflight_report':str(target)}
        sync_suite_evidence(self.bundle,report)
        self.assertEqual(report['cases'][identity]['conformance_status'],'pass','Known successful preflight evidence must survive publication')
        self.assertEqual(report['cases'][identity]['artifacts'],{'candidate':record['pdf']})
        record['assertions'][0]['status']='fail'
        write_report(target,{'cases':{identity:{'status':'blocked','record':record}}})
        sync_suite_evidence(self.bundle,report)
        self.assertEqual(report['cases'][identity]['conformance_status'],'fail','A failed preflight assertion must survive case evaluation')

    def test_filtered_run_cannot_claim_full_conformance(self):
        selected=[self.ids[0]];self.report['cases']={selected[0]:self.report['cases'][selected[0]]}
        self.report['selected_case_ids']=selected
        gates=gate_statuses(self.bundle,self.report,selected)
        self.assertNotEqual(gates['conformance'],'pass')
        self.assertEqual(gates['selected_conformance'],'pass')

    def test_unverified_requirement_blocks_conformance(self):
        self.assertEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')
        self.bundle['requirements'][TEST_REQUIREMENT_ID]['verification']=[]
        self.report['profile_sha256']=profile_fingerprint(self.bundle)
        self.assertNotEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')

    def test_all_execution_failures_keep_selected_denominator(self):
        states=['unsupported','render_error','timeout','infrastructure_error','not_run']
        for identity,status in zip(self.ids,states):self.report['cases'][identity].update(status=status)
        self.report['cases'].pop(self.ids[-1])
        summary=build_summary(self.bundle,self.report)
        self.assertEqual(summary['selected'],18);self.assertEqual(summary['not_run'],2)
        self.assertEqual(sum(summary['execution'].values()),18)
        self.assertNotEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')

    def test_optional_absence_is_distinct_from_required_infrastructure_failure(self):
        self.report['optional_tools']['weasyprint']='unavailable'
        self.assertEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')
        self.report['infrastructure_errors']=['pdffonts executable unavailable']
        self.assertNotEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')

    def test_changed_locked_asset_blocks_conformance(self):
        self.assertEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')
        self.report['cases'][self.ids[0]]['asset_lock_valid']=False
        self.assertNotEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')

    def test_profile_fingerprint_source_closure_and_interruption_block(self):
        for mutate in [lambda:self.report.update(profile_sha256='b'*64),lambda:self.report.update(complete=False),
                       lambda:self.report['provenance'].update(source_commit=None),lambda:self.bundle['sources']['css-sizing-3'].update(reference_dispositions=[{'status':'unresolved','url':'unknown'}])]:
            with self.subTest(mutation=mutate):
                old=json.loads(json.dumps(self.report));old_source=dict(self.bundle['sources']['css-sizing-3'])
                mutate();self.assertNotEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')
                self.report=old;self.bundle['sources']['css-sizing-3']=old_source

    def test_real_calibration_metadata_false_is_not_a_failed_check(self):
        from comparison import calibrate,default_settings
        self.report['comparison_calibration']['checks']=calibrate(self.root/'calibration',default_settings())
        self.assertFalse(self.report['comparison_calibration']['checks']['synthetic_glyph_for_textless_input'])
        self.assertEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')
        self.report['comparison_calibration']['checks']['missing_glyph_rejected']=False
        self.assertNotEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')

    def test_suite_preflight_supports_renderer_without_version_command(self):
        from test_support import write_probe_pdf
        pdf=write_probe_pdf(self.root/'source.pdf')
        executable=self.root/'renderer'
        executable.write_text('#!'+sys.executable+'\nimport sys\nfrom pathlib import Path\nif len(sys.argv)!=3: raise SystemExit(1)\nPath(sys.argv[2]).write_bytes(Path('+repr(str(pdf))+').read_bytes())\n')
        executable.chmod(0o755)
        output=self.root/'preflight'
        completed=subprocess.run([sys.executable,str(Path(__file__).with_name('run.py')),'--benchmark-suite','--preflight-only','--case','bench-reports-small','--binary',str(executable),'--output-dir',str(output)],capture_output=True,text=True)
        self.assertEqual(completed.returncode,0,completed.stdout+completed.stderr)
        result=json.loads((output/'report.json').read_text())
        self.assertTrue(result['complete']);self.assertFalse(result['infrastructure_errors'])
        self.assertEqual(result['renderer']['version_status'],'unavailable')
        self.assertEqual(result['cases']['bench-reports-small']['status'],'rendered')
        self.assertNotEqual(result['gates']['selected_conformance'],'pass','The probe PDF lacks the workload content')

    def test_binary_override_has_unknown_build_properties(self):
        from types import SimpleNamespace
        from run import run_provenance
        result=run_provenance(SimpleNamespace(binary=Path(sys.executable)))
        self.assertIsNone(result['build']['profile'])
        self.assertIsNone(result['build']['locked'])
        self.assertIsNone(result['build']['features'])
        self.assertFalse(result['build']['verified'])

    def test_changed_tolerances_require_matching_calibration(self):
        self.assertEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')
        self.bundle['defaults']['tolerances']['glyph_origin_pt']=0.1
        self.report['profile_sha256']=profile_fingerprint(self.bundle)
        self.assertNotEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')
        self.report['comparison_calibration']['settings_sha256']=calibration_fingerprint(self.bundle['defaults']['tolerances'])
        self.assertEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')
        self.report.pop('comparison_calibration')
        self.assertNotEqual(gate_statuses(self.bundle,self.report,self.ids)['conformance'],'pass')

    def test_markdown_links_spaces_unicode_and_partial_json_roundtrip(self):
        directory=self.root/'output spaces Ω';directory.mkdir()
        artifact=directory/'PDF space Ω.pdf';artifact.write_bytes(b'probe')
        self.report.update(complete=False,historical=True)
        self.report['cases'][self.ids[0]]['artifacts']={'candidate':str(artifact)}
        self.report['summary']=build_summary(self.bundle,self.report)
        self.report['gates']=gate_statuses(self.bundle,self.report,self.ids)
        write_report(directory/'report.json',self.report)
        loaded=json.loads((directory/'report.json').read_text());self.assertFalse(loaded['complete'])
        page=write_markdown(loaded,directory)
        text=page.read_text();self.assertIn('Historical',text);self.assertIn('Incomplete',text)
        for link in re.findall(r'\]\(([^)]+)\)',text):
            if link.startswith(('http:','https:','#')):continue
            self.assertTrue((directory/unquote(link.split('#')[0])).exists(),link)
        self.assertIn('%20',text);self.assertIn('requirements.md',text)


if __name__=='__main__':unittest.main()
