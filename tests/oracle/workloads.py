"""Deterministic local document workloads and correctness eligibility."""
import html
import hashlib
from io import BytesIO
from pathlib import Path
import shutil
import uuid

from PIL import Image
from comparison import AnalysisError, analyse
from conformance import check_assertions, inspect_fonts
from execution import COMMAND_INTERRUPTED, RENDERED, CommandError, classify_render, digest, read_log, render_diagnostics, run_command, write_report

HERE = Path(__file__).resolve().parent
FONT_FILES = ('DejaVuSans.ttf','DejaVuSans-Bold.ttf','STIXGeneral.ttf')
FONT_LICENCES = ('FONT-LICENSE.txt','STIX-LICENSE.txt')
IMAGE_EDGE_PX = 24
WORKLOAD_VERSION = 'static-documents-v2'


class WorkloadError(ValueError):
    pass


class WorkloadInterrupted(KeyboardInterrupt):
    def __init__(self,record):
        self.record = record


def image_bytes(index):
    image = Image.new('RGB',(IMAGE_EDGE_PX,IMAGE_EDGE_PX))
    seed = str(index).encode('ascii')
    image.putdata([tuple(hashlib.sha256(seed+b':'+str(y*IMAGE_EDGE_PX+x).encode('ascii')).digest()[:3])
                   for y in range(IMAGE_EDGE_PX) for x in range(IMAGE_EDGE_PX)])
    stream = BytesIO()
    image.save(stream,format='PNG',compress_level=9)
    return stream.getvalue()


def _document(case):
    config = case['generator']
    family,count = config['family'],config['count']
    css = """@font-face{font-family:Bench;src:url(assets/DejaVuSans.ttf);font-weight:400}
@font-face{font-family:Bench;src:url(assets/DejaVuSans-Bold.ttf);font-weight:700}
@page{size:595pt 842pt;margin:54pt 42pt 42pt;font-family:Bench;font-size:10pt}
html,body{margin:0;padding:0}body{font:10pt/14pt Bench}h1{font-size:18pt;line-height:24pt}h2{font-size:14pt;line-height:20pt}
p{margin:0 0 10pt}table{border-collapse:collapse;width:100%}th,td{padding:4pt;border:0.5pt solid #334455;text-align:left}
thead{display:table-header-group}tfoot{display:table-footer-group}tr{break-inside:avoid}
"""
    title = f'{family.title()} {case["scale"]}'
    body = f'<h1>{html.escape(title)}</h1>'
    if family=='invoices':
        body += '<p>Invoice INV-2026-001. Customer Example Research.</p><table><thead><tr><th>Item</th><th>Quantity</th><th>Amount</th></tr></thead><tbody>'
        for index in range(1,count+1):
            body += f'<tr><td>Item-{index:05d}</td><td>{index%7+1}</td><td>{index*137//100}.{index*137%100:02d}</td></tr>'
        total = sum(index*137 for index in range(1,count+1))
        body += f'</tbody></table><p>Invoice total: {total//100}.{total%100:02d}</p>'
    elif family=='reports':
        css += 'section+section{break-before:page}'
        body = ''
        for index in range(1,count+1):
            body += f'<section><h1>Report section {index:04d}</h1>'
            for paragraph in range(1,7):
                body += f'<p>Finding-{index:04d}-{paragraph:02d}: Consistent measurements connect the observed result with its evidence. Data is retained for review, including uncertainty and the final decision.</p>'
            body += '</section>'
    elif family=='books':
        css += """@page{@top-left{content:string(chapter)}@bottom-center{content:'Page ' counter(page)}}
h1{string-set:chapter content()}section{break-before:page}nav a{display:block}nav a::after{content:leader('.') target-counter(attr(href),page)}"""
        body = '<h1>Contents</h1><nav>'+''.join(f'<a href="#chapter-{index}">Chapter {index:03d}</a>' for index in range(1,count+1))+'</nav>'
        for index in range(1,count+1):
            body += f'<section><h1 id="chapter-{index}">Chapter {index:03d}</h1><p>Cross reference to <a href="#chapter-{count}">the final chapter</a>.</p>'
            body += ''.join(f'<p>Passage-{index:03d}-{paragraph:02d}: A book needs consistent chapter structure, running content, page numbers, and resolved cross references. The text remains available to search and copy.</p>' for paragraph in range(1,9))+'</section>'
    elif family=='multilingual':
        css += '@font-face{font-family:Primary;src:url(assets/STIXGeneral.ttf)}section p{font-family:Primary,Bench}'
        languages = (('en','Office affinity AVATAR 0123456789'),('el','Ελληνικά Καλημέρα κόσμε'),
                     ('ru','Кириллица Доброе утро'),('ar','العربية مرحبا بالعالم'),('he','עברית שלום עולם'))
        for index in range(1,count+1):
            body += f'<section><h2>Language group {index:04d}</h2>'
            for language,text in languages:
                direction = 'rtl' if language in ('ar','he') else 'ltr'
                body += f'<p lang="{language}" dir="{direction}">{text} <span dir="ltr">Record-{index:04d}-{language}</span></p>'
            body += '</section>'
    elif family=='tables':
        body += '<table><thead><tr><th>Ledger header</th><th colspan="2">Quarterly figures</th></tr></thead><tbody>'
        for index in range(1,count+1):
            if index%2:
                body += f'<tr><td rowspan="2">Group-{index:05d}</td><td>Row-{index:05d}</td><td>{index*11}</td></tr>'
            else:
                body += f'<tr><td colspan="2">Row-{index:05d} combined amount {index*11}</td></tr>'
        body += f'</tbody><tfoot><tr><td colspan="3">Ledger complete {count:05d}</td></tr></tfoot></table>'
    elif family=='images':
        css += '.tile{display:inline-block;width:108pt;margin:4pt;vertical-align:top}.tile img{display:block;width:96pt;height:48pt}figure{margin:0}'
        for index in range(count):
            body += f'<figure class="tile"><img src="assets/unique-{index:04d}.png" alt="Unique pattern"><img src="assets/reused.png" alt="Reused raster"><img src="assets/vector.svg" alt="Reused vector"><figcaption>Image-{index+1:05d}</figcaption></figure>'
    else:
        raise WorkloadError(f'Unknown workload family {family!r}')
    return f'<!doctype html><html lang="en"><head><meta charset="utf-8"><title>{title}</title><style>{css}</style></head><body>{body}</body></html>\n'


def materialise_workloads(bundle: dict, output_dir: Path) -> dict[str,Path]:
    directory = Path(output_dir)/'workloads'
    assets = directory/'assets'
    assets.mkdir(parents=True,exist_ok=True)
    for name in (*FONT_FILES,*FONT_LICENCES):
        shutil.copyfile(HERE/'assets'/name,assets/name)
    (assets/'reused.png').write_bytes(image_bytes(0))
    (assets/'vector.svg').write_text('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 48"><rect width="96" height="48" fill="#ddeeff"/><circle cx="24" cy="24" r="18" fill="#224466"/><path d="M48 42L70 6L92 42Z" fill="#bb3311"/></svg>\n')
    result = {}
    identities = [identity for group in bundle['benchmarks']['classes'] for identity in group['case_ids']]
    for identity in identities:
        case = bundle['cases'][identity]
        if case['generator']['version']!=WORKLOAD_VERSION:
            raise WorkloadError(f'{identity}: unknown workload version')
        if case['generator']['family']=='images':
            for index in range(case['generator']['count']):
                (assets/f'unique-{index:04d}.png').write_bytes(image_bytes(index+1))
        path = directory/f'{identity}.html'
        path.write_text(_document(case),encoding='utf-8')
        expected = case.get('provenance',{}).get('input_sha256')
        if expected and digest(path)!=expected:
            raise WorkloadError(f'{identity}: generated input differs from committed lock')
        for resource in case.get('resources',[]):
            if digest(path.parent/resource['path'])!=resource['sha256']:
                raise WorkloadError(f'{identity}: asset differs from committed lock: {resource["path"]}')
        result[identity] = path
    return result


def workload_verifier(case,path,bundle,output_dir):
    def verify(pdf):
        inspected=verify_workload(case,path,pdf,bundle,Path(output_dir)/pdf.stem)
        checked=check_eligibility(case,inspected)
        return {'status':'pass' if checked['status']=='eligible' else 'blocked','inspection':inspected,'reasons':checked['reasons']}
    return verify


def resource_hashes(case,path):
    return {resource['path']:digest(path.parent/resource['path']) for resource in case.get('resources',[])}


def check_eligibility(case: dict, record: dict) -> dict:
    reasons = []
    expected_resources = {item['path']:item['sha256'] for item in case.get('resources',[])}
    expected_input = case.get('provenance',{}).get('input_sha256')
    if not case.get('assertions'):
        reasons.append('No independent output assertions')
    if record.get('status')!=RENDERED:
        reasons.append(f'execution {record.get("status","not_run")}')
    if not expected_input or record.get('input_sha256')!=expected_input:
        reasons.append('input fingerprint differs from suite lock')
    if record.get('resources_sha256')!=expected_resources:
        reasons.append('asset fingerprint differs from suite lock')
    assertions = {item['id']:item for item in record.get('assertions',[])}
    for assertion in case.get('assertions',[]):
        if assertion.get('mandatory',True) and assertions.get(assertion['id'],{}).get('status')!='pass':
            reasons.append(f'assertion {assertion["id"]} {assertions.get(assertion["id"],{}).get("status","missing")}')
    if record.get('warnings_unresolved'):
        reasons.append('unresolved render warnings')
    return {'status':'blocked' if reasons else 'eligible','reasons':reasons,'record':record}


def render_workload(case,path,executable,bundle,output_dir):
    directory = Path(output_dir)/case['id']
    directory.mkdir(parents=True,exist_ok=True)
    pdf = directory/f'{uuid.uuid4().hex}.pdf'
    execution = run_command([executable,path,pdf],directory/'logs',case['timeout_s'])
    record = verify_workload(case,path,pdf,bundle,directory,status=classify_render(execution,pdf),execution=execution)
    if execution['status']==COMMAND_INTERRUPTED:
        raise WorkloadInterrupted(record)
    return record


def verify_workload(case,path,pdf,bundle,directory,*,status=RENDERED,execution=None):
    record = {'status':status,'execution':execution,'pdf':str(pdf),
              'input_sha256':digest(path),'resources_sha256':resource_hashes(case,path),'assertions':[]}
    try:
        artifacts = {'candidate':pdf,'status':record['status']}
        record['assertions'] = check_assertions(case,artifacts,bundle,directory/'assertions')
        if record['status']==RENDERED:
            if 'analysis' not in artifacts:artifacts['analysis']=analyse(pdf,settings=bundle['defaults']['tolerances'],output_dir=directory/'analysis',timeout_s=case['timeout_s'])
            record['pdf_sha256'] = digest(pdf)
            record['pdf_bytes'] = pdf.stat().st_size
            record['pages'] = len(artifacts['analysis']['pages'])
            record['fonts'] = artifacts['fonts'] if 'fonts' in artifacts else inspect_fonts(pdf,directory,case['timeout_s'])
        log = read_log(execution,'stderr') if execution else ''
        record.update(render_diagnostics(log,case.get('warnings',[])))
    except (CommandError,AnalysisError,OSError,ValueError) as error:
        record.update(status='infrastructure_error',error=str(error))
    return record


def preflight_workloads(bundle,paths,binary,output_dir):
    report = {'schema_version':1,'complete':False,'cases':{identity:{'status':'not_run'} for identity in paths}}
    target = Path(output_dir)/'workload-preflight.json'
    write_report(target,report)
    try:
        for identity,path in paths.items():
            case = bundle['cases'][identity]
            report['cases'][identity] = {'status':'incomplete'}
            write_report(target,report)
            try:
                record = render_workload(case,path,binary,bundle,Path(output_dir)/'preflight')
            except WorkloadInterrupted as error:
                report['cases'][identity] = {'status':'incomplete','record':error.record}
                raise
            report['cases'][identity] = check_eligibility(case,record)
            write_report(target,report)
        report['complete'] = True
    finally:
        write_report(target,report)
    return report
