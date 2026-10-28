#!/usr/bin/env python3
"""Render the oracle profile with the Rust CLI, check independent output assertions and optionally compare local WeasyPrint output."""
from comparison import AnalysisError, analyse, compare_pdfs
from conformance import check_assertions, evaluate_case, evaluate_requirements
from reporting import calibrate_tolerances, EXIT_CONFORMANCE_FAILURE, build_summary, gate_statuses, profile_fingerprint, write_markdown
import argparse
import json
import uuid
from pathlib import Path
from execution import COMMAND_INTERRUPTED, DEFAULT_COMMAND_TIMEOUT_S, CommandError, RENDERED, NOT_RUN, INFRASTRUCTURE_ERROR, RENDER_ERROR, REPORT_SCHEMA_VERSION, EXIT_FAILURE, EXIT_INTERRUPTED, capture_provenance, checked_command, classify_render, digest, read_log, render_diagnostics, run_command, write_report
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
OUTPUT = HERE / 'output'
BINARY = ROOT / 'target/release/ferri-pdf'
SOURCE_PATHS = [ROOT/'Cargo.toml',ROOT/'Cargo.lock',ROOT/'build.rs',ROOT/'build',ROOT/'src',ROOT/'spec',ROOT/'assets',ROOT/'vendor',HERE]


def run_provenance(args):
    provenance = capture_provenance(ROOT,SOURCE_PATHS)
    provenance['build'] = {'profile':None if args.binary else 'release','locked':None if args.binary else True,'binary_override':args.binary is not None,'verified':args.binary is None,
                           'command':None if args.binary else ['cargo','build','--release','--locked','--manifest-path',str(ROOT/'Cargo.toml')],
                           'features':None if args.binary else 'workspace defaults'}
    provenance['toolchain'] = {tool:checked_command(command,output_dir=OUTPUT/'logs',timeout_s=DEFAULT_COMMAND_TIMEOUT_S)[0]
                               for tool,command in [('rustc',['rustc','-vV']),('cargo',['cargo','--version'])]}
    provenance['analysis_tools'] = {tool:checked_command([tool,'-v'],output_dir=OUTPUT/'logs',timeout_s=DEFAULT_COMMAND_TIMEOUT_S)[1]
                                    for tool in ('pdftoppm','pdftotext','pdffonts')}
    return provenance


def sync_suite_evidence(bundle,report):
    preflight_path = report.get('preflight_report')
    if preflight_path and Path(preflight_path).is_file():
        preflight = json.loads(Path(preflight_path).read_text())
        for identity,eligibility in preflight['cases'].items():
            record = eligibility.get('record')
            if record is None:continue
            report['cases'][identity] = {**record,'case_id':identity,'eligibility':eligibility,
                'conformance_status':evaluate_case(bundle['cases'][identity],record),
                'artifacts':{'candidate':record['pdf']} if record.get('pdf') else {},
                'requirement_ids':bundle['cases'][identity]['requirement_ids']}
    benchmark_path = report.get('benchmark_report')
    if benchmark_path and Path(benchmark_path).is_file():
        report['benchmark'] = json.loads(Path(benchmark_path).read_text())


def publish_report(bundle, report, output_dir):
    if report.get('scope')=='benchmark-suite':sync_suite_evidence(bundle,report)
    report['requirements'] = evaluate_requirements(bundle,report['cases'])
    report['summary'] = build_summary(bundle,report)
    report['gates'] = gate_statuses(bundle,report,report['selected_case_ids'])
    write_report(Path(output_dir)/'report.json',report)
    write_markdown(report,output_dir)


def enforce_gates(args,report):
    conformance_gate = 'foundations' if getattr(args,'foundations',False) else 'conformance' if report.get('scope')=='full-profile' else 'selected_conformance'
    if args.require_conformance and report['gates'][conformance_gate]!='pass':
        raise SystemExit(EXIT_CONFORMANCE_FAILURE)


def main():
    global OUTPUT
    parser = argparse.ArgumentParser()
    parser.add_argument('--require-conformance', action='store_true', help='require every selected mandatory standards obligation; unfiltered means the complete profile')
    parser.add_argument('--benchmark-suite', action='store_true', help='run the locked representative suite')
    parser.add_argument('--preflight-only', action='store_true', help='render eligibility checks without timing')
    parser.add_argument('--case', action='append')
    parser.add_argument('--output-dir', type=Path, help='directory for candidate PDFs, rasters and report')
    parser.add_argument('--weasyprint', type=Path, help='optional local WeasyPrint executable for independent secondary comparisons')
    parser.add_argument('--binary', type=Path, help='check this prebuilt renderer instead of building target/release')
    parser.add_argument('--manifest-dir', type=Path, default=HERE / 'manifests', help='versioned oracle manifest directory')
    parser.add_argument('--validate-manifests', action='store_true', help='validate manifests without rendering')
    parser.add_argument('--inventory', action='store_true', help='report the print standards inventory without rendering')
    parser.add_argument('--foundations', action='store_true', help='evaluate the complete frozen foundations scope separately from full release conformance')
    args = parser.parse_args()
    if args.foundations and args.benchmark_suite:
        parser.error('--foundations cannot be combined with benchmark modes')
    full_profile = args.require_conformance and not args.case and not args.benchmark_suite and not args.foundations
    if args.validate_manifests or args.inventory:
        from manifest import ManifestError, inventory_summary, load_bundle
        try:
            bundle = load_bundle(args.manifest_dir, scope='inventory' if args.inventory else 'full')
            if args.foundations and "foundations" not in bundle:
                from foundations import load_foundations
                bundle["foundations"] = load_foundations(bundle)
        except ManifestError as error:
            parser.error(str(error))
        if args.inventory:
            summary = inventory_summary(bundle)
            if args.foundations:
                from foundations import evaluate_foundations
                summary['foundations'] = evaluate_foundations(bundle,bundle['foundations'],{})
            print(json.dumps(summary, indent=2))
        else:
            print(f"Manifests valid: {len(bundle['requirements'])} requirements, {len(bundle['cases'])} cases")
            if args.foundations:print(f"Foundations valid: {len(bundle['foundations']['behaviors'])} behaviors")
        return
    from manifest import ManifestError, load_bundle, suite_case_ids
    try:
        bundle = load_bundle(args.manifest_dir, scope='full' if args.foundations or args.benchmark_suite or full_profile else 'cases')
        if args.foundations and "foundations" not in bundle:
            from foundations import load_foundations
            bundle["foundations"] = load_foundations(bundle)
    except ManifestError as error:
        parser.error(str(error))
    settings = bundle['defaults']['tolerances']
    optional_tools = {'weasyprint': 'not_requested'}
    if args.weasyprint:
        args.weasyprint = args.weasyprint.resolve()
        optional_tools['weasyprint'] = 'available' if args.weasyprint.is_file() else 'unavailable'
        if not args.weasyprint.is_file():
            args.weasyprint = None
    if args.output_dir:
        OUTPUT = args.output_dir.resolve()
    if args.preflight_only and not args.benchmark_suite:
        parser.error('--preflight-only requires --benchmark-suite')
    if args.benchmark_suite:
        from workloads import WorkloadError, materialise_workloads, preflight_workloads, workload_verifier
        OUTPUT.mkdir(parents=True, exist_ok=True)
        suite_report = {'schema_version':REPORT_SCHEMA_VERSION,'run_id':uuid.uuid4().hex,'complete':False,'cases':{},'selected_case_ids':[identity for identity in suite_case_ids(bundle) if not args.case or identity in args.case], 'infrastructure_errors':[],'profile_sha256':profile_fingerprint(bundle),'optional_tools':optional_tools,'filtered':bool(args.case),'scope':'benchmark-suite'}
        suite_report['cases'] = {identity:{'case_id':identity,'status':NOT_RUN} for identity in suite_report['selected_case_ids']}
        write_report(OUTPUT/'report.json',suite_report)
        try:
            paths = materialise_workloads(bundle, OUTPUT)
            if args.case:
                if not set(args.case).issubset(paths):
                    parser.error('unknown benchmark case ID')
                paths = {identity:path for identity,path in paths.items() if identity in args.case}
            binary = args.binary.resolve() if args.binary else BINARY
            if not args.binary:
                checked_command(['cargo','build','--release','--locked','--manifest-path',ROOT/'Cargo.toml'], output_dir=OUTPUT/'logs', timeout_s=DEFAULT_COMMAND_TIMEOUT_S)
            suite_report['provenance'] = run_provenance(args)
            suite_report['provenance']['build']['binary_sha256'] = digest(binary)
            probe = run_command([binary,'--version'],OUTPUT/'logs',DEFAULT_COMMAND_TIMEOUT_S)
            available = probe['status']=='completed' and probe['exit_status']==0
            suite_report['renderer'] = {'path':str(binary),'sha256':digest(binary),'version':read_log(probe,'stdout') if available else None,'version_status':'available' if available else 'unavailable','version_probe':probe}
            suite_report['comparison_calibration'] = calibrate_tolerances(bundle,OUTPUT/'calibration')
            from benchmark import fingerprints
            fingerprint_paths = [*SOURCE_PATHS,*paths.values(),*[path.parent/resource['path'] for identity,path in paths.items() for resource in bundle['cases'][identity].get('resources',[])]]
            fingerprint_case = {'fingerprint_paths':fingerprint_paths}
            preflight_fingerprints = fingerprints(fingerprint_case,[binary])
            suite_report['preflight_fingerprints'] = preflight_fingerprints
            suite_report['preflight_report'] = str(OUTPUT/'workload-preflight.json')
            write_report(OUTPUT/'report.json',suite_report)
            preflight = preflight_workloads(bundle, paths, binary, OUTPUT)
            sync_suite_evidence(bundle,suite_report)
            print(f"Workload preflight: {sum(record['status']=='eligible' for record in preflight['cases'].values())}/{len(paths)} eligible; {OUTPUT / 'workload-preflight.json'}")
            if not args.preflight_only:
                from benchmark import benchmark_suite
                entries = {}
                for identity,path in paths.items():
                    case = bundle['cases'][identity]
                    verify = workload_verifier(case,path,bundle,OUTPUT/'timed-inspection'/identity)
                    entries[identity] = {'case':{**case,'fingerprint_paths':fingerprint_paths,'expected_fingerprints':preflight_fingerprints},
                        'command':[str(binary),str(path),'{output}'],'eligibility':preflight['cases'][identity],'verify':verify}
                suite_report['benchmark_report'] = str(OUTPUT/'measurements/benchmark-suite.json')
                write_report(OUTPUT/'report.json',suite_report)
                measured = benchmark_suite(bundle,entries,OUTPUT/'measurements')
                suite_report['benchmark'] = measured
                print(f"Benchmark timings: {measured['aggregate']['status']}; {OUTPUT / 'measurements/benchmark-suite.json'}")
            suite_report['complete'] = True
        except KeyboardInterrupt:
            suite_report['interrupted'] = True
            publish_report(bundle,suite_report,OUTPUT)
            raise SystemExit(EXIT_INTERRUPTED)
        except (WorkloadError,CommandError,AnalysisError,OSError) as error:
            suite_report['infrastructure_errors'].append(str(error))
            publish_report(bundle,suite_report,OUTPUT)
            parser.exit(EXIT_FAILURE, f'Workload execution failed: {error}\n')
        publish_report(bundle,suite_report,OUTPUT)
        enforce_gates(args,suite_report)
        return
    if args.foundations:
        from foundations import required_case_ids
        fixtures = {identity:ROOT/bundle["cases"][identity]["input"]
                    for identity in required_case_ids(bundle,bundle["foundations"])}
    else:
        fixtures = {identity:ROOT/definition['input'] for identity,definition in bundle['cases'].items() if definition.get('input')}
    if full_profile:
        from workloads import materialise_workloads
        fixtures.update(materialise_workloads(bundle,OUTPUT))
    OUTPUT.mkdir(parents=True, exist_ok=True)
    if args.case and (not set(args.case).issubset(fixtures)):
        parser.error('unknown case ID')
    selected_names = [name for name in fixtures if not args.case or name in args.case]
    run_id = uuid.uuid4().hex
    report = {'schema_version': REPORT_SCHEMA_VERSION, 'run_id': run_id, 'complete': False, 'cases': {name: {'case_id': name, 'status': NOT_RUN} for name in selected_names}, 'selected_case_ids': selected_names, 'infrastructure_errors': [],'optional_tools':optional_tools,'profile_sha256':profile_fingerprint(bundle),'filtered':bool(args.case),'scope':'foundations' if args.foundations else 'full-profile' if full_profile else 'diagnostic'}
    if args.foundations and not selected_names:
        report["complete"] = True
        report["provenance"] = capture_provenance(ROOT,SOURCE_PATHS)
        report["provenance"]["build"] = {"verified":False,"reason":"No declared foundations output evidence to render."}
        publish_report(bundle,report,OUTPUT)
        enforce_gates(args,report)
        return
    write_report(OUTPUT / 'report.json', report)
    try:
        if args.binary:
            binary = args.binary.resolve()
        else:
            checked_command(['cargo', 'build', '--release', '--locked', '--manifest-path', ROOT / 'Cargo.toml'], output_dir=OUTPUT / 'logs', timeout_s=DEFAULT_COMMAND_TIMEOUT_S)
            binary = BINARY
        report.update({'candidate': {'path': str(binary), 'sha256': digest(binary)}, 'tolerances': settings})
        report['provenance'] = run_provenance(args)
    except (CommandError, OSError) as error:
        report['infrastructure_errors'].append(str(error))
        publish_report(bundle,report,OUTPUT)
        parser.exit(EXIT_FAILURE, f'Preflight failed: {error}\n')
    if args.weasyprint:
        try:
            report['secondary_renderer'] = {'name': 'WeasyPrint', 'path': str(args.weasyprint), 'launcher_sha256': digest(args.weasyprint), 'version': checked_command([args.weasyprint, '--version'], output_dir=OUTPUT / 'logs', timeout_s=DEFAULT_COMMAND_TIMEOUT_S)[0].strip(), 'environment': checked_command([args.weasyprint, '--info'], output_dir=OUTPUT / 'logs', timeout_s=DEFAULT_COMMAND_TIMEOUT_S)[0]}
            report['secondary_cases'] = {}
        except (CommandError,OSError) as error:
            report['optional_tools']['weasyprint'] = 'unavailable'
            report['optional_tool_errors'] = {'weasyprint':str(error)}
            args.weasyprint = None
    source_root = HERE / 'assets'
    report['input_asset_root'] = str(source_root)
    report['input_assets_sha256'] = {str(source.relative_to(source_root)): digest(source) for source in sorted(source_root.rglob('*')) if source.is_file()}
    for name in selected_names:
        source = fixtures[name]
        definition = bundle['cases'][name]
        case_timeout_s = definition['timeout_s']
        candidate = OUTPUT / f'{run_id}-{name}.rust.pdf'
        try:
            rendered = run_command([str(binary), str(source), str(candidate), '--trace'], OUTPUT / 'logs', case_timeout_s)
            if rendered['status'] == COMMAND_INTERRUPTED:
                raise KeyboardInterrupt
            execution_status = classify_render(rendered, candidate)
            if execution_status != RENDERED:
                report['cases'][name] = {'case_id': name, 'status': execution_status, 'error': read_log(rendered, 'stderr') or rendered.get('error', ''), 'execution': rendered}
                print(f"{name}: {execution_status}", flush=True)
                continue
            candidate_analysis = analyse(candidate, settings=settings, output_dir=OUTPUT, timeout_s=case_timeout_s)
            result = {'case_id': name, 'status': RENDERED, 'execution': rendered, 'artifacts': {'candidate': str(candidate)}, 'pages': len(candidate_analysis['pages'])}
            if args.weasyprint:
                secondary = OUTPUT / f'{run_id}-{name}.weasyprint.pdf'
                try:
                    _, secondary_log = checked_command([args.weasyprint, source, secondary], output_dir=OUTPUT / 'logs', timeout_s=DEFAULT_COMMAND_TIMEOUT_S)
                    report['secondary_cases'][name] = {'pdf_sha256': digest(secondary), 'render_log': secondary_log, 'rust_comparison': compare_pdfs(secondary, candidate, f'{name}.weasyprint-rust', candidate_analysis=candidate_analysis, settings=settings, output_dir=OUTPUT, timeout_s=case_timeout_s)}
                except (CommandError,AnalysisError,OSError) as error:
                    report['secondary_cases'][name] = {'status':INFRASTRUCTURE_ERROR,'error':str(error),'rust_comparison':None}
            result['font_report'] = checked_command(['pdffonts', candidate], output_dir=OUTPUT / 'logs', timeout_s=DEFAULT_COMMAND_TIMEOUT_S)[0]
            result['assertions'] = check_assertions(definition, {'candidate': candidate, 'status': RENDERED, 'analysis': candidate_analysis}, bundle, OUTPUT / 'assertions' / name)
            result['conformance_status'] = evaluate_case(definition, result)
            report['cases'][name] = result
            print(f"{name}: pages={result['pages']}, assertions passed={sum(item['status']=='pass' for item in result['assertions'])}/{len(result['assertions'])}, conformance={result['conformance_status']}", flush=True)
        except KeyboardInterrupt:
            report['interrupted'] = True
            report['cases'][name] = {'case_id': name, 'status': RENDER_ERROR, 'error': 'rendering interrupted'}
            publish_report(bundle,report,OUTPUT)
            raise SystemExit(EXIT_INTERRUPTED)
        except (CommandError, AnalysisError, OSError) as error:
            report['cases'][name] = {'case_id': name, 'status': INFRASTRUCTURE_ERROR, 'error': str(error)}
            print(f'{name}: infrastructure error: {error}', flush=True)
        finally:
            record = report['cases'][name]
            record['requirement_ids'] = definition['requirement_ids']
            expected_input = definition.get('provenance',{}).get('input_sha256')
            record['input_sha256'] = digest(source) if source.is_file() else None
            record['input_lock_valid'] = record['input_sha256']==expected_input if expected_input else None
            from workloads import resource_hashes
            try:
                record['resources_sha256']=resource_hashes(definition,source)
                expected_resources={item['path']:item['sha256'] for item in definition.get('resources',[])}
                record['asset_lock_valid']=record['resources_sha256']==expected_resources if expected_resources else None
            except OSError as error:
                record['asset_lock_valid']=False
                record['asset_lock_error']=str(error)
            record.update(render_diagnostics(read_log(record['execution'],'stderr') if 'execution' in record else '',definition.get('warnings',[])))
            if 'assertions' not in record:
                record['assertions'] = check_assertions(definition, {'candidate': candidate, 'status': record['status']}, bundle, OUTPUT / 'assertions' / name)
                record['conformance_status'] = evaluate_case(definition, record)
            write_report(OUTPUT / 'report.json', report)
    report['requirements'] = evaluate_requirements(bundle, report['cases'])
    try:
        report['comparison_calibration'] = calibrate_tolerances(bundle,OUTPUT/'calibration')
    except (AnalysisError,CommandError,OSError) as error:
        report['infrastructure_errors'].append(str(error))
        publish_report(bundle,report,OUTPUT)
        parser.exit(EXIT_FAILURE,f'Calibration failed: {error}\n')
    if args.weasyprint:
        secondary_cases = report['secondary_cases'].values()
        report['secondary_summary'] = {'selected': len(report['secondary_cases']), 'compared': sum(case['rust_comparison'] is not None for case in secondary_cases), 'matching': sum(case['rust_comparison'] is not None and case['rust_comparison']['match'] for case in secondary_cases)}
    report['complete'] = True
    publish_report(bundle,report,OUTPUT)
    print(f"Report: {OUTPUT / 'report.json'}", flush=True)
    enforce_gates(args,report)
if __name__ == '__main__':
    main()
