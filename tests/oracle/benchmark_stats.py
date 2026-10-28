"""Fixed-suite timing statistics; no machine or file access."""
import math
import statistics


def _duration(value):
    return not isinstance(value,bool) and isinstance(value,(int,float)) and math.isfinite(value) and value>0


def aggregate(samples: dict[str,list[dict]], suite: dict, eligibility: dict[str,dict]) -> dict:
    protocol = suite.get('protocol',{})
    identities = [identity for group in suite.get('classes',[]) for identity in group.get('case_ids',[])]
    result = {'status':'blocked','documents':{},'missing_case_ids':[],'reasons':[],'protocol':dict(protocol)}
    for identity in identities:
        runs = samples.get(identity,[])
        document = {'status':'blocked','observations':len(runs),'median_ms':None,'minimum_ms':None,'maximum_ms':None,'reasons':[]}
        result['documents'][identity] = document
        problems = document['reasons']
        if not runs:
            result['missing_case_ids'].append(identity)
        if len(runs)!=protocol.get('timed_runs'):
            problems.append(f'Requires {protocol.get("timed_runs")} runs; retained {len(runs)}')
        if eligibility.get(identity,{}).get('status')!='eligible':
            problems.append('Correctness evidence is not eligible')
        if [run.get('run_index') for run in runs]!=list(range(len(runs))):
            problems.append('Run indices must be complete and ordered without replacements')
        for index,run in enumerate(runs):
            if run.get('status')!='valid' or not _duration(run.get('elapsed_ms')):
                problems.append(f'Run {index}: invalid outcome or duration')
        if problems:
            result['reasons'].append(f'{identity}: '+'; '.join(problems))
            continue
        durations = [run['elapsed_ms'] for run in runs]
        document.update(status='valid',median_ms=statistics.median(durations),minimum_ms=min(durations),maximum_ms=max(durations))
    result['status'] = 'blocked' if result['reasons'] else 'valid'
    return result
