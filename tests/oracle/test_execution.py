import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from test_support import write_probe_pdf


class ExecutionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    def run_child(self, code, timeout=5):
        from execution import run_command
        return run_command([sys.executable, '-c', code], self.root/'logs', timeout)

    def test_failed_child_cannot_reuse_previous_pdf(self):
        from execution import classify_render
        write_probe_pdf(self.root/'previous.pdf')
        record = self.run_child('raise SystemExit(1)')
        self.assertEqual(classify_render(record, self.root/'attempt.pdf'), 'render_error')
        self.assertEqual(record['exit_status'], 1)

    def test_only_explicit_capability_diagnostic_is_unsupported(self):
        from execution import classify_render
        samples = [
            ('CSS: unsupported CSS: grid-template-columns: 1fr 2fr', 'unsupported'),
            ('resource error: failed to open grid.html', 'render_error'),
            ('resource error: bad file\nCSS: unsupported CSS: forged', 'render_error'),
            ('thread main panicked at unsupported CSS', 'render_error'),
        ]
        for diagnostic, expected in samples:
            with self.subTest(diagnostic=diagnostic):
                code = f'import sys; sys.stderr.write({diagnostic!r}); raise SystemExit(1)'
                record = self.run_child(code)
                self.assertEqual(classify_render(record, self.root/'missing.pdf'), expected)

    def test_success_requires_a_fresh_valid_pdf_artifact(self):
        from execution import classify_render
        source = write_probe_pdf(self.root/'source.pdf')
        output = self.root/'render.pdf'
        record = self.run_child(f'from pathlib import Path; Path({str(output)!r}).write_bytes(Path({str(source)!r}).read_bytes())')
        self.assertEqual(classify_render(record, output), 'rendered')
        missing = self.run_child('pass')
        self.assertEqual(classify_render(missing, self.root/'absent.pdf'), 'render_error')

    def test_missing_executable_is_infrastructure_not_unsupported(self):
        from execution import classify_render, run_command
        record = run_command([str(self.root/'missing-executable')], self.root/'logs', 1)
        self.assertEqual(classify_render(record, self.root/'none.pdf'), 'infrastructure_error')

    def test_timeout_reaps_the_process_and_captures_its_diagnostics(self):
        from execution import classify_render
        record = self.run_child('import sys,time; print("started",flush=True); time.sleep(30)', timeout=0.1)
        self.assertEqual(classify_render(record, self.root/'none.pdf'), 'timeout')
        self.assertIn('started', Path(record['stdout_path']).read_text())
        self.assertLess(record['elapsed_ms'], 5000)
        with self.assertRaises(ProcessLookupError):
            os.kill(record['pid'], 0)

    def test_timeout_stops_a_grandchild_that_ignores_termination(self):
        pid_file = self.root/'grandchild.pid'
        heartbeat = self.root/'heartbeat'
        child = f'import os,signal,time; from pathlib import Path; signal.signal(signal.SIGTERM,signal.SIG_IGN); Path({str(pid_file)!r}).write_text(str(os.getpid())); p=Path({str(heartbeat)!r}); exec("while True:\\n p.write_text(str(time.time_ns()))\\n time.sleep(0.01)")'
        code = f'import subprocess,sys,time; subprocess.Popen([sys.executable,"-c",{child!r}]); time.sleep(30)'
        record = self.run_child(code,timeout=0.3)
        self.assertEqual(record['status'],'timeout')
        self.assertTrue(pid_file.exists())
        pid = int(pid_file.read_text())
        def cleanup_child():
            try:
                os.kill(pid,signal.SIGKILL)
            except ProcessLookupError:
                pass
        self.addCleanup(cleanup_child)
        before = heartbeat.read_text()
        time.sleep(0.08)
        self.assertEqual(heartbeat.read_text(),before,'grandchild survived timeout cleanup')

    def test_timed_interval_uses_blocking_wait_not_timeout_polling(self):
        original = subprocess.Popen.wait
        observed = []
        def observe(process,*args,**kwargs):
            observed.append(kwargs.get('timeout',args[0] if args else None))
            return original(process,*args,**kwargs)
        with patch.object(subprocess.Popen,'wait',observe):
            result = self.run_child('pass')
        self.assertEqual(result['exit_status'],0)
        self.assertIsNone(observed[0],'Popen.wait(timeout) adds polling sleeps to measured rendering')

    def test_previous_artifact_at_the_same_path_is_not_fresh_output(self):
        from execution import classify_render
        output = write_probe_pdf(self.root/'stale.pdf')
        old = time.time()-60
        os.utime(output,(old,old))
        record = self.run_child('pass')
        self.assertEqual(classify_render(record,output),'render_error')

    def test_source_and_untracked_resource_changes_invalidate_fingerprints(self):
        from execution import capture_provenance
        subprocess.run(['git','init','-q',str(self.root/'repo')],check=True)
        repo = self.root/'repo'
        tracked = repo/'source.txt';tracked.write_text('first')
        subprocess.run(['git','-C',str(repo),'add','source.txt'],check=True)
        resource = repo/'font.bin';resource.write_bytes(b'font-one')
        first = capture_provenance(repo, [tracked,resource])
        resource.write_bytes(b'font-two')
        second = capture_provenance(repo, [tracked,resource])
        self.assertNotEqual(first['files'], second['files'])
        tracked.write_text('second')
        third = capture_provenance(repo, [tracked,resource])
        self.assertNotEqual(second['dirty_diff_sha256'], third['dirty_diff_sha256'])

    def test_atomic_report_preserves_selected_case_accounting(self):
        from execution import write_report
        path = self.root/'reports/report.json'
        report = {'cases': {'complete': {'status': 'rendered'}, 'pending': {'status': 'not_run'}}, 'complete': False}
        write_report(path,report)
        self.assertEqual(json.loads(path.read_text()),report)
        self.assertEqual(list(path.parent.glob('*.tmp')),[])

    def test_real_cli_preflight_failure_still_writes_all_selected_cases(self):
        output = self.root/'preflight'
        result = subprocess.run([sys.executable,str(Path(__file__).with_name('run.py')),'--case','latin','--case','paint-overlap','--binary',str(self.root/'absent'),'--output-dir',str(output)],capture_output=True,text=True)
        self.assertNotEqual(result.returncode,0)
        self.assertTrue((output/'report.json').exists(),result.stderr)
        report = json.loads((output/'report.json').read_text())
        self.assertEqual(set(report['cases']),{'latin','paint-overlap'})
        self.assertFalse(report['complete'])
        self.assertTrue(report['infrastructure_errors'])

    def test_suite_interruption_publishes_partial_preflight_and_active_execution(self):
        script=Path(__file__).with_name('run.py')
        source=write_probe_pdf(self.root/'probe.pdf');marker=self.root/'suite-attempts'
        executable=self.root/'controlled-suite-renderer'
        executable.write_text(f'#!{sys.executable}\nimport sys,time\nfrom pathlib import Path\nif "--version" in sys.argv: raise SystemExit(1)\np=Path({str(marker)!r})\nn=int(p.read_text())+1 if p.exists() else 1\np.write_text(str(n))\nif n==2: time.sleep(30)\nPath(sys.argv[2]).write_bytes(Path({str(source)!r}).read_bytes())\n')
        executable.chmod(0o755);output=self.root/'suite-interrupt'
        process=subprocess.Popen([sys.executable,str(script),'--benchmark-suite','--preflight-only','--case','bench-invoices-small','--case','bench-reports-small','--case','bench-images-small','--binary',str(executable),'--output-dir',str(output)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        self.addCleanup(lambda:process.kill() if process.poll() is None else None)
        deadline=time.monotonic()+20
        while time.monotonic()<deadline and process.poll() is None:
            if marker.exists() and marker.read_text()=='2':break
            time.sleep(.02)
        self.assertTrue(marker.exists(),'renderer never started')
        self.assertEqual(marker.read_text(),'2')
        process.send_signal(signal.SIGINT);stdout,stderr=process.communicate(timeout=5)
        self.assertEqual(process.returncode,130,stdout+stderr)
        report=json.loads((output/'report.json').read_text())
        self.assertEqual(report['cases']['bench-invoices-small']['status'],'rendered')
        self.assertEqual(report['cases']['bench-images-small']['status'],'not_run')
        preflight=json.loads(Path(report['preflight_report']).read_text())
        active=preflight['cases']['bench-reports-small']['record']
        self.assertEqual(active['execution']['status'],'interrupted')
        self.assertFalse(report['complete'])
        self.assertIn('workload-preflight.json',(output/'report.md').read_text())

    def test_real_cli_interruption_keeps_finished_and_pending_cases(self):
        script = Path(__file__).with_name('run.py')
        source = write_probe_pdf(self.root/'probe.pdf')
        marker = self.root/'attempts'
        executable = self.root/'controlled-renderer'
        executable.write_text(f'#!{sys.executable}\nimport sys,time\nfrom pathlib import Path\np=Path({str(marker)!r})\nn=int(p.read_text())+1 if p.exists() else 1\np.write_text(str(n))\nif n==2: time.sleep(30)\nPath(sys.argv[2]).write_bytes(Path({str(source)!r}).read_bytes())\n')
        executable.chmod(0o755)
        output = self.root/'interrupt'
        process = subprocess.Popen([sys.executable,str(script),'--case','latin','--case','paint-overlap','--case','vertical-align-keywords','--binary',str(executable),'--output-dir',str(output)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        self.addCleanup(lambda: process.kill() if process.poll() is None else None)
        deadline = time.monotonic()+15
        while time.monotonic()<deadline and process.poll() is None:
            if marker.exists() and marker.read_text()=='2':
                break
            time.sleep(0.02)
        self.assertTrue(marker.exists(),'renderer was not started')
        self.assertEqual(marker.read_text(),'2')
        process.send_signal(signal.SIGINT)
        stdout,stderr = process.communicate(timeout=5)
        self.assertEqual(process.returncode,130,stderr+'\n'+stdout)
        report = json.loads((output/'report.json').read_text())
        self.assertEqual(set(report['cases']),{'latin','paint-overlap','vertical-align-keywords'})
        self.assertFalse(report['complete'])
        statuses = [case['status'] for case in report['cases'].values()]
        self.assertIn('rendered',statuses)
        self.assertIn('not_run',statuses)
        self.assertTrue(report.get('interrupted',False))


if __name__ == '__main__':
    unittest.main()
