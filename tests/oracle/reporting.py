"""Independent conformance gates and linked human-readable execution evidence."""
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
from urllib.parse import quote

from comparison import calibration_valid
from conformance import FAIL, BLOCKED, combine, evaluate_case, evaluate_requirements
from manifest import inventory_summary

EXIT_CONFORMANCE_FAILURE = 3
EXECUTION_STATES = ('rendered','unsupported','render_error','timeout','infrastructure_error','not_run')
PROFILE_COLLECTIONS = ('sources','sections','requirements','cases','benchmarks','defaults')


def profile_fingerprint(bundle):
    content = {name:bundle[name] for name in PROFILE_COLLECTIONS}
    if "foundations" in bundle:content["foundations"] = bundle["foundations"]
    return hashlib.sha256(json.dumps(content,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()


def _records(report):
    return {record.get('case_id') or identity:record for identity,record in report.get('cases',{}).items()}


def _selection(report):
    return report.get('selected_case_ids',list(_records(report)))


def build_summary(bundle: dict, report: dict) -> dict:
    records = _records(report)
    selected = _selection(report)
    execution = Counter(records.get(identity,{}).get('status','not_run') for identity in selected)
    requirements = evaluate_requirements(bundle,records)
    inventory = inventory_summary(bundle)
    return {'selected':len(selected),'unsupported':execution['unsupported'],'not_run':execution['not_run'],
            'execution':{state:execution[state] for state in EXECUTION_STATES},
            'requirements':dict(Counter(record['status'] for record in requirements.values())),
            'implementation':inventory['implementation'],'requirements_total':inventory['requirements_total'],
            'source_closure_complete':inventory['source_closure_complete'],'unresolved_imports':len(inventory['unresolved_imports']),
            'unresolved_overlaps':len(inventory['unresolved_overlaps']),'inventory_complete':inventory['inventory_complete'],'unclassified_case_ids':[identity for identity in selected if identity not in bundle['cases']]}


def calibration_fingerprint(settings):
    return hashlib.sha256(json.dumps(settings,sort_keys=True,separators=(',',':'),allow_nan=False).encode()).hexdigest()


def calibrate_tolerances(bundle,output_dir):
    from comparison import calibrate
    settings = bundle['defaults']['tolerances']
    return {'settings_sha256':calibration_fingerprint(settings),'checks':calibrate(Path(output_dir),settings)}


def gate_statuses(bundle: dict, report: dict, selected_ids: list[str]) -> dict:
    records = _records(report)
    selected = list(dict.fromkeys(selected_ids))
    requirements = evaluate_requirements(bundle,records)
    selected_requirements = {identity for case_id in selected for identity in bundle['cases'].get(case_id,{}).get('requirement_ids',[])}
    statuses = [requirements[identity]['status'] for identity in selected_requirements if requirements[identity]['status']!='not_applicable']
    for identity in selected:
        definition = bundle['cases'].get(identity)
        if records.get(identity,{}).get('input_lock_valid') is False or records.get(identity,{}).get('asset_lock_valid') is False or records.get(identity,{}).get('warnings_unresolved'):
            statuses.append(BLOCKED)
        if definition is None:
            statuses.append('unverified')
        else:
            statuses.append(evaluate_case(definition,records.get(identity,{})))
    local = combine(statuses)
    problems = []
    if not report.get('complete'):
        problems.append('Run is incomplete')
    if report.get('infrastructure_errors') or any(records.get(identity,{}).get('status')=='infrastructure_error' for identity in selected):
        problems.append('Required infrastructure failed')
    if report.get('profile_sha256')!=profile_fingerprint(bundle):
        problems.append('Source/profile fingerprint is absent or changed')
    provenance = report.get('provenance',{})
    if not provenance.get('source_commit') or not provenance.get('files') or any(value is None for value in provenance.get('files',{}).values()):
        problems.append('Required source/resource provenance is incomplete')
    calibration = report.get('comparison_calibration',{})
    if selected and (calibration.get('settings_sha256')!=calibration_fingerprint(bundle['defaults']['tolerances']) or not calibration_valid(calibration.get('checks',{}))):
        problems.append('Required comparison calibration is missing, changed or failed')
    if problems:
        local = FAIL if local==FAIL else BLOCKED
    full = set(selected)==set(bundle['cases']) and not report.get('filtered',False) and all(identity in records for identity in selected)
    inventory = inventory_summary(bundle)
    global_states = [record['status'] for record in requirements.values() if record['status']!='not_applicable']
    conformance = combine([local,*global_states])
    if not full or not inventory['inventory_complete'] or problems:
        conformance = FAIL if conformance==FAIL else BLOCKED
    gates = {'selected_conformance':local,'conformance':conformance,
            'full_profile':full,'selected_case_ids':selected,'missing_case_ids':sorted(set(bundle['cases'])-set(selected)),
            'infrastructure_blockers':problems}
    if "foundations" in bundle:
        from foundations import evaluate_foundations
        evidence = evaluate_foundations(bundle,bundle["foundations"],records)
        gates["foundations_evidence"] = evidence
        gates["foundations"] = evidence["status"]
        if problems and gates["foundations"] != FAIL:gates["foundations"] = BLOCKED
    return gates


def _cell(value):
    return str(value).replace('|','\\|').replace('\n',' ')


def _link(label,path,directory):
    relative = os.path.relpath(Path(path).resolve(),directory.resolve())
    return f'[{_cell(label)}]({quote(relative,safe="/.-_")})'


def write_markdown(report: dict, output_dir: Path) -> Path:
    directory = Path(output_dir)
    directory.mkdir(parents=True,exist_ok=True)
    target = directory/'report.md'
    label = 'Historical evidence' if report.get('historical') else 'Re-evaluated execution evidence' if report.get('postprocessing') else 'Current execution evidence'
    lines = [f'# {label}', '', 'Complete run.' if report.get('complete') else '**Incomplete run: no overall pass.**','',
             _link('Raw JSON',directory/'report.json',directory)+' · [Requirement evidence](requirements.md)','']
    preflight_file = report.get('preflight_report')
    if preflight_file and Path(preflight_file).exists():lines += [_link('Workload preflight',preflight_file,directory),'']
    lineage=report.get('postprocessing',{})
    if lineage:
        lines += [_cell(lineage.get('reason','Re-evaluated recorded artifacts.')),'',f'Acquisition revision: `{lineage.get("acquisition_source_commit","recorded in raw JSON")}`; evaluation revision: `{lineage.get("processing_source_commit","recorded in raw JSON")}`.','']
        acquisition=lineage.get('acquisition_report',{}).get('path')
        if acquisition and Path(acquisition).exists():lines += [_link('Original acquisition JSON',acquisition,directory),'']
    lines += ['| Gate | Status |','| --- | --- |']
    for gate in ('selected_conformance','conformance',*(['foundations'] if 'foundations' in report.get('gates',{}) else [])):
        lines.append(f'| {gate} | {report.get("gates",{}).get(gate,"blocked")} |')
    foundations = report.get('gates',{}).get('foundations_evidence')
    if foundations:
        lines += ["", "Foundations scoped source closure: "+str(foundations["source_closure_complete"])+".",
                  "", "| Foundations behavior | Status | Owner task |", "| --- | --- | --- |"]
        lines += [f'| {_cell(identity)} | {item["status"]} | {item["owner_task"]} |'
                  for identity,item in foundations["behaviors"].items()]
        lines += ["", "Unverified scoped requirements: "+str(len(foundations["unverified_requirement_ids"]))+"."]
    summary = report.get('summary',{})
    lines += ['',f'Selected: {summary.get("selected",len(report.get("cases",{})))}. Source closure: {summary.get("source_closure_complete",False)}. Unresolved imports: {summary.get("unresolved_imports","unknown")}; unresolved overlaps: {summary.get("unresolved_overlaps","unknown")}.',
              '', '| Execution | Count |','| --- | --- |']
    lines += [f'| {state} | {count} |' for state,count in summary.get('execution',{}).items()]
    lines += ['', 'Implementation evidence: '+_cell(summary.get('implementation',{})), '',
              'Requirement verification: '+_cell(summary.get('requirements',{})), '',
              '## Case evidence','']
    records = _records(report)
    for identity,record in records.items():
        lines += [f'<a id="case-{quote(identity,safe=".-_")}"></a>',f'### {_cell(identity)}','',
                  f'Execution: {record.get("status","not_run")}; conformance: {record.get("conformance_status","unverified")}.','']
        for name,path in record.get('artifacts',{}).items():
            if path and Path(path).exists():lines.append(_link(name,path,directory))
        if record.get('error'):lines += ['',_cell(record['error'])]
        lines += ['', '| Assertion | Status | Diagnostics |','| --- | --- | --- |']
        lines += [f'| {_cell(item["id"])} | {item["status"]} | {_cell(item.get("diagnostics",[]))} |' for item in record.get('assertions',[])]
        for requirement in record.get('requirement_ids',[]):
            lines.append(f'[Requirement {_cell(requirement)}](requirements.md#{quote(requirement,safe=".-_")})')
        lines.append('')
    benchmark = report.get('benchmark',{})
    if benchmark:
        lines += ['## Benchmark timings','',_cell(benchmark.get('aggregate',{})),'']
        timings_file = report.get('benchmark_report')
        if timings_file and Path(timings_file).exists():lines += [_link('Raw timings, eligibility and RSS results',timings_file,directory),'']
    for blocker in report.get('gates',{}).get('infrastructure_blockers',[]):lines.append('- '+_cell(blocker))
    target.write_text('\n'.join(lines)+'\n')
    evidence = ['# Requirement evidence','', '[Execution report](report.md)','',
                'Inventory implementation state does not establish verified conformance. Exclusions do not add passes.','']
    for identity,record in report.get('requirements',{}).items():
        evidence += [f'<a id="{quote(identity,safe=".-_")}"></a>',f'## {_cell(identity)}','',f'Status: {record["status"]}.','']
        if record.get('source_url'):
            evidence += [f'[Pinned source]({record["source_url"]})','']
        for case_id in record.get('case_ids',[]):
            if case_id in records:evidence.append(f'[Case {_cell(case_id)}](report.md#case-{quote(case_id,safe=".-_")})')
        for assertion in record.get('assertions',[]):evidence.append(_cell(assertion))
        if not record.get('assertions'):evidence.append('Missing independent mandatory assertion evidence.')
        evidence.append('')
    (directory/'requirements.md').write_text('\n'.join(evidence)+'\n')
    return target
