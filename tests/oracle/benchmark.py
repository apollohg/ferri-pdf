"""Sequential CLI timings with post-render correctness checks."""
import json
from pathlib import Path
from typing import Callable
import uuid

from benchmark_stats import aggregate
from comparison import AnalysisError
from execution import COMMAND_INTERRUPTED, RENDERED, CommandError, classify_render, digest, fingerprint_files, read_log, render_diagnostics, run_command, write_report

OUTPUT_PLACEHOLDER = '{output}'
VALID_VERIFICATION = ('pass','eligible')


class AttemptInterrupted(KeyboardInterrupt):
    def __init__(self,record):
        self.record = record


def fingerprints(case,command):
    paths = set(map(str,case.get('fingerprint_paths',[])))
    paths.add(str(command[0]))
    return fingerprint_files(sorted(paths))


def _attempt(case,command,directory,verify,*,rss=False):
    pdf = directory/f'{uuid.uuid4().hex}.pdf'
    argv = [str(token).replace(OUTPUT_PLACEHOLDER,str(pdf)) for token in command]
    execution = run_command(argv,directory/'logs',case['timeout_s'],collect_rss=rss)
    record = {'status':classify_render(execution,pdf),'execution':execution,'pdf':str(pdf),
              'verification':{'status':'blocked'},'pdf_bytes':None}
    if execution['status']==COMMAND_INTERRUPTED:
        raise AttemptInterrupted(record)
    if record['status']==RENDERED:
        record.update(pdf_bytes=pdf.stat().st_size,pdf_sha256=digest(pdf))
        try:
            record['verification'] = verify(pdf)
        except KeyboardInterrupt:
            record['verification'] = {'status':'blocked','interrupted':True}
            raise AttemptInterrupted(record)
        except (CommandError,AnalysisError,OSError,ValueError) as error:
            record['verification'] = {'status':'blocked','error':str(error),'infrastructure_error':True}
    record.update(render_diagnostics(read_log(execution,'stderr'),case.get('warnings',[])))
    if record['warnings_unresolved']:
        record['verification']['status'] = 'blocked'
    if rss:
        record['peak_rss_bytes'] = execution.get('peak_rss_bytes')
        record['rss_unavailable_reason'] = execution.get('rss_unavailable_reason')
    return record


def _valid(record):
    return record['status']==RENDERED and record['verification'].get('status') in VALID_VERIFICATION


def benchmark_case(case: dict, command: list[str], protocol: dict, output_dir: Path,
                   verify: Callable[[Path],dict]) -> dict:
    directory = Path(output_dir)
    directory.mkdir(parents=True,exist_ok=True)
    if not command or not any(OUTPUT_PLACEHOLDER in str(token) for token in command) or '--trace' in command:
        raise ValueError('Benchmark command requires an output placeholder and no trace flag')
    for field in ('warmup_runs','timed_runs'):
        if isinstance(protocol.get(field),bool) or not isinstance(protocol.get(field),int) or protocol[field]<0:
            raise ValueError(f'Invalid benchmark protocol {field}')
    result = {'status':'blocked','case_id':case['id'],'warmups':[],'runs':[],'rss':None,'reasons':[],
              'eligibility':case.get('eligibility'),'fingerprints_before':fingerprints(case,command),
              'fingerprints_expected':case.get('expected_fingerprints'),'fingerprints_after':None,'protocol':protocol,'interrupted':False}
    target = directory/'benchmark-case.json'
    write_report(target,result)
    try:
        if result['fingerprints_expected'] is not None and result['fingerprints_expected']!=result['fingerprints_before']:
            result['reasons'].append('Input, asset, source or executable differs from preflight fingerprints')
            return result
        for stage,count in (('warmups',protocol['warmup_runs']),('runs',protocol['timed_runs'])):
            for index in range(count):
                run = {'run_index':index,'status':'incomplete'}
                result[stage].append(run)
                try:
                    run['attempt'] = _attempt(case,command,directory,verify)
                except AttemptInterrupted as error:
                    run['attempt'] = error.record
                    raise
                run['elapsed_ms'] = run['attempt']['execution']['elapsed_ms']
                run['status'] = 'valid' if _valid(run['attempt']) else 'invalid'
                if run['status']!='valid':
                    result['reasons'].append(f'{stage} run {index}: failed execution or output verification')
                write_report(target,result)
        try:
            result['rss'] = _attempt(case,command,directory,verify,rss=True)
        except AttemptInterrupted as error:
            result['rss'] = error.record
            raise
        if not _valid(result['rss']):
            result['reasons'].append('RSS observation: failed execution or output verification')
        write_report(target,result)
        if protocol['timed_runs']==0:
            result['reasons'].append('No timed runs requested')
    except KeyboardInterrupt:
        result['interrupted'] = True
        result['reasons'].append('Interrupted; incomplete observations retained without replacement')
        raise
    finally:
        result['fingerprints_after'] = fingerprints(case,command)
        if result['fingerprints_before']!=result['fingerprints_after']:
            result['reasons'].append('Input, asset or executable fingerprint changed during measurement')
        if any(value is None for value in result['fingerprints_before'].values()):
            result['reasons'].append('Required fingerprint unavailable')
        result['status'] = 'blocked' if result['reasons'] else 'valid'
        write_report(target,result)
    return result


def benchmark_suite(bundle: dict, cases: dict, output_dir: Path) -> dict:
    directory = Path(output_dir)
    directory.mkdir(parents=True,exist_ok=True)
    from manifest import suite_case_ids
    identities = suite_case_ids(bundle)
    report = {'schema_version':1,'complete':False,'cases':{identity:{'status':'not_run','runs':[]} for identity in identities},'aggregate':None}
    target = directory/'benchmark-suite.json'
    write_report(target,report)
    try:
        for identity in identities:
            entry = cases.get(identity)
            if not entry or entry.get('eligibility',{}).get('status')!='eligible':
                report['cases'][identity] = {'status':'blocked','runs':[],'eligibility':entry.get('eligibility') if entry else None,
                    'reasons':['Preflight correctness is not eligible; no timings attempted']}
            else:
                try:
                    report['cases'][identity] = benchmark_case({**entry['case'],'eligibility':entry['eligibility']},entry['command'],
                                                               bundle['benchmarks']['protocol'],directory/identity,entry['verify'])
                except KeyboardInterrupt:
                    report['cases'][identity] = json.loads((directory/identity/'benchmark-case.json').read_text())
                    raise
            write_report(target,report)
        report['complete'] = True
    finally:
        eligibility = {identity:{'status':'eligible' if record['status']=='valid' else 'blocked'} for identity,record in report['cases'].items()}
        report['aggregate'] = aggregate({identity:record['runs'] for identity,record in report['cases'].items()},bundle['benchmarks'],eligibility)
        write_report(target,report)
    return report
