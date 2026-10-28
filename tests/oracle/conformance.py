"""Independent PDF assertions and source-backed requirement verification."""
import math
from collections import Counter
from pathlib import Path
import re
import unicodedata

from PIL import Image, ImageChops
import pdf_content
from comparison import analyse, compare_pdfs, raster
from execution import checked_command, digest

PASS = 'pass'
FAIL = 'fail'
UNVERIFIED = 'unverified'
BLOCKED = 'blocked'
NOT_APPLICABLE = 'not_applicable'
RASTERS_PER_VECTOR = 2
FONT_ROW = re.compile(r'^(\S+)\s+.*?\s+(yes|no)\s+(yes|no)\s+(yes|no)\s+\d+\s+\d+\s*$')


def combine(statuses, *, missing=UNVERIFIED):
    values = list(statuses)
    for status in (FAIL, BLOCKED, UNVERIFIED):
        if status in values:
            return status
    return PASS if values and all(value == PASS for value in values) else missing


def inspect_fonts(pdf, output_dir, timeout_s):
    report = checked_command(['pdffonts',pdf],output_dir=Path(output_dir)/'logs',timeout_s=timeout_s)[0]
    records = []
    for line in report.splitlines()[2:]:
        if not line.strip():
            continue
        match = FONT_ROW.fullmatch(line)
        if not match:
            raise ValueError(f'Unrecognised pdffonts row: {line}')
        records.append({'name':pdf_content.canonical_font_name(match[1]),'embedded':match[2]=='yes',
                        'subset':match[3]=='yes','unicode':match[4]=='yes'})
    return records


def _text(value):
    return ''.join(unicodedata.normalize('NFC',value).split())


def _glyphs(analysis):
    return [item['glyph'] for line in analysis['glyph_lines'] for item in line]


def _basis_valid(basis,bundle):
    if not isinstance(basis,dict) or basis.get('ambiguous') or not basis.get('reason'):
        return False
    if basis.get('kind') == 'specification':
        source_id = basis.get('source_id')
        requested = basis.get('sections')
        if not isinstance(source_id,str) or not isinstance(requested,list) or not requested or any(
                not isinstance(section,str) for section in requested):
            return False
        source = bundle['sources'].get(source_id)
        sections = {item['id'] for item in bundle['sections'].get(source_id,[])}
        return bool(source and basis.get('sha256') == source['sha256']
                    and set(requested) <= sections)
    if basis.get('kind') in ('independent-reference','upstream-test'):
        return bool(basis.get('sha256') and basis.get('licence') and basis.get('origin'))
    return False


def _measure(assertion,artifacts,bundle,output_dir,settings):
    kind,expected = assertion['kind'],assertion['expected']
    analysis = artifacts['analysis']
    pages = analysis['pages']
    tolerance = settings['glyph_origin_pt']
    if kind == 'page-count':
        return len(pages),len(pages)==expected
    if kind == 'page-boxes':
        actual = [{'width_pt':page['width'],'height_pt':page['height']} for page in pages]
        target = expected if isinstance(expected,list) else [expected]*len(pages)
        return actual,len(actual)==len(target) and all(
            abs(page[key]-size[key])<=tolerance for page,size in zip(actual,target) for key in ('width_pt','height_pt'))
    if kind == 'unicode-text':
        if 'per_page' in expected:
            measurements = [_measure({**assertion,'expected':item},artifacts,bundle,output_dir,settings) for item in expected['per_page']]
            return [actual for actual,_ in measurements],bool(measurements) and all(matches for _,matches in measurements)
        texts = expected.get('patterns',expected.get('texts',[expected.get('text','')]))
        if 'page_texts' not in artifacts:
            artifacts['page_texts'] = {str(number):_text(''.join(glyph.text for glyph in _glyphs(analysis) if glyph.page==number))
                                      for number in range(1,len(pages)+1)}
        targets = expected.get('pages','all')
        selected = list(range(1,len(pages)+1)) if targets in ('all','any') else targets
        desired = expected.get('count_per_page')
        actual = {}
        matches = bool(texts and selected)
        for text in texts:
            needle = _text(text)
            counts = {number:len(re.findall(text,page)) if 'patterns' in expected else page.count(needle) for number,page in artifacts['page_texts'].items()}
            checks = [str(number) in counts and (counts[str(number)]==desired if desired is not None else counts[str(number)]>0)
                      for number in selected]
            matches &= bool(needle) and (any(checks) if targets=='any' else all(checks))
            if expected.get('only_pages'):
                matches &= all(not count for number,count in counts.items() if int(number) not in selected)
            actual[text] = counts
        return actual[texts[0]] if 'text' in expected else actual,matches
    if kind == 'word-counts':
        actual = Counter(text for page in pages for word in page['words']
                         for text in word.get('visual_words', [word['text']]))
        missing = {word:{'expected_minimum':count,'actual':actual[word]} for word,count in expected['minimum_counts'].items() if actual[word]<count}
        return {'missing':missing,'observed_words':sum(actual.values())},not missing
    if kind == 'image-placements':
        return _measure_images(expected,artifacts,output_dir,settings)
    if kind == 'glyphs':
        glyphs = [glyph for glyph in _glyphs(analysis) if 'page' not in expected or glyph.page==expected['page']]
        if 'codepoint_ranges' in expected:
            glyphs = [glyph for glyph in glyphs if any(low<=ord(character)<=high for character in glyph.text for low,high in expected['codepoint_ranges'])]
        actual = [{'text':glyph.text,'font':glyph.font,'origin_pt':list(glyph.origin),
                   'fill':list(glyph.fill),'render_mode':glyph.render_mode} for glyph in glyphs]
        matches = len(glyphs)>=expected.get('minimum_glyphs',1)
        if 'text' in expected:
            matches &= ''.join(glyph.text for glyph in glyphs)==expected['text']
        for field,attribute in (('font','font'),('render_mode','render_mode')):
            if field in expected:
                matches &= all(getattr(glyph,attribute)==expected[field] for glyph in glyphs)
        if 'fill' in expected:
            matches &= all(len(glyph.fill)==len(expected['fill']) and all(abs(a-b)<=settings['glyph_colour']
                           for a,b in zip(glyph.fill,expected['fill'])) for glyph in glyphs)
        if 'origins_pt' in expected:
            matches &= len(glyphs)==len(expected['origins_pt']) and all(
                abs(a-b)<=tolerance for glyph,origin in zip(glyphs,expected['origins_pt']) for a,b in zip(glyph.origin,origin))
        return actual,matches
    if kind == 'fonts':
        if 'fonts' not in artifacts:
            artifacts['fonts'] = inspect_fonts(artifacts['candidate'],output_dir,artifacts['timeout_s'])
        actual = artifacts['fonts']
        names = {font['name'] for font in actual}
        matches = bool(names) and names==set(expected['names'])
        if 'embedded' in expected:
            matches &= all(font['embedded']==expected['embedded'] for font in actual)
        return actual,matches
    if kind == 'region-ink':
        if 'rasters' not in artifacts:
            artifacts['rasters'] = raster(artifacts['candidate'],settings=settings,output_dir=output_dir,timeout_s=artifacts['timeout_s'])
        number = expected['page']
        if number < 1 or number > len(artifacts['rasters']):
            return {'page':number,'dark_pixels':None},False
        scale = settings['resolution_dpi']/settings['points_per_inch']
        rectangle = tuple(round(value*scale) for value in expected['rectangle_pt'])
        with Image.open(artifacts['rasters'][number-1]) as image:
            if rectangle[0]<0 or rectangle[1]<0 or rectangle[2]>image.width or rectangle[3]>image.height:
                return {'page':number,'rectangle_outside_page':True},False
            channels = image.convert('RGB').crop(rectangle).split()
            maximum = ImageChops.lighter(ImageChops.lighter(channels[0],channels[1]),channels[2])
            count = sum(maximum.histogram()[:settings['regional_dark_threshold']])
        return {'page':number,'dark_pixels':count},expected.get('minimum_dark_pixels',0)<=count<=expected.get('maximum_dark_pixels',math.inf)
    if kind in ('pdf-reference','raster-reference'):
        reference = Path(bundle['root'])/expected['path']
        if not reference.is_file() or digest(reference)!=expected['sha256']:
            return {'reference':str(reference),'hash_valid':False},None
        comparison = compare_pdfs(reference,artifacts['candidate'],assertion['id'],output_dir=output_dir,
            settings=settings,timeout_s=artifacts['timeout_s'])
        return comparison,comparison['match' if kind=='pdf-reference' else 'visual_match']
    return None,None


def _measure_images(expected,artifacts,output_dir,settings):
    analysis = artifacts['analysis']
    images = analysis.get('images',[])
    expected_hashes = expected['raster_pixels_sha256']
    problems = []
    if len(images)!=len(expected_hashes):problems.append('Raster placement count differs; omitted rasters or rasterised vectors are not equivalent work')
    tolerance = settings['glyph_origin_pt']
    width,height = expected['size_pt']
    for index,(image,pixels_hash) in enumerate(zip(images,expected_hashes)):
        if image.get('pixels_sha256')!=pixels_hash:problems.append(f'Raster {index}: source pixels differ or encoding cannot be verified')
        rectangle=image['rectangle_pt']
        if abs(rectangle[2]-rectangle[0]-width)>tolerance or abs(rectangle[3]-rectangle[1]-height)>tolerance:problems.append(f'Raster {index}: placement size differs')
    if 'rasters' not in artifacts:
        artifacts['rasters']=raster(artifacts['candidate'],settings=settings,output_dir=output_dir,timeout_s=artifacts['timeout_s'])
    scale=settings['resolution_dpi']/settings['points_per_inch']
    samples=[]
    for index in range(0,len(images)-1,RASTERS_PER_VECTOR):
        unique,reused=images[index:index+RASTERS_PER_VECTOR]
        left,top,right,bottom=unique['rectangle_pt']
        expected_reused=[left,top+height,right,bottom+height]
        if unique['page']!=reused['page'] or any(abs(a-b)>tolerance for a,b in zip(expected_reused,reused['rectangle_pt'])):
            problems.append(f'Tile {index//RASTERS_PER_VECTOR}: raster pair geometry differs')
            continue
        page=unique['page']
        if not 1<=page<=len(artifacts['rasters']):
            problems.append(f'Tile {index//RASTERS_PER_VECTOR}: missing raster page')
            continue
        with Image.open(artifacts['rasters'][page-1]) as image:
            for sample in expected['vector_samples']:
                x=round((left+sample['offset_pt'][0])*scale)
                y=round((top+RASTERS_PER_VECTOR*height+sample['offset_pt'][1])*scale)
                actual=list(image.convert('RGB').getpixel((x,y))) if 0<=x<image.width and 0<=y<image.height else None
                valid=actual is not None and all(abs(a-b)<=settings['pixel_delta'] for a,b in zip(actual,sample['rgb']))
                samples.append({'tile':index//RASTERS_PER_VECTOR,'page':page,'pixel':[x,y],'rgb':actual,'match':valid})
                if not valid:problems.append(f'Tile {index//RASTERS_PER_VECTOR}: missing or incorrect vector paint at {sample["offset_pt"]}')
    return {'raster_placements':images,'vector_samples':samples,'diagnostics':problems},not problems


def check_assertions(case: dict, artifacts: dict, bundle: dict, output_dir: Path) -> list[dict]:
    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True,exist_ok=True)
    pdf = Path(artifacts.get('candidate',''))
    artifacts.setdefault('timeout_s',case['timeout_s'])
    settings = bundle['defaults']['tolerances']
    results = []
    for assertion in case['assertions']:
        record = {'id':assertion['id'],'status':UNVERIFIED,'expected':assertion.get('expected'),
                  'actual':None,'evidence':{'pdf':str(pdf)},'diagnostics':[]}
        results.append(record)
        if artifacts.get('status','rendered')!='rendered' or not pdf.is_file():
            record.update(status=BLOCKED,diagnostics=['Candidate output unavailable: '+artifacts.get('status','missing output')])
            continue
        if not _basis_valid(assertion.get('basis'),bundle):
            record['diagnostics'].append('Missing, ambiguous or mismatched independent source basis')
            continue
        if 'analysis' not in artifacts:
            artifacts['analysis'] = analyse(pdf,settings=settings,output_dir=output_dir,timeout_s=artifacts['timeout_s'])
        actual,matches = _measure(assertion,artifacts,bundle,output_dir,settings)
        record.update(actual=actual,status=UNVERIFIED if matches is None else PASS if matches else FAIL)
        record['evidence']['pdf_sha256'] = digest(pdf)
        record['evidence']['basis'] = assertion['basis']
        if matches is None:
            record['diagnostics'].append('Unsupported assertion kind or missing independent reference evidence')
    return results


def evaluate_case(case,record):
    assertions = {item['id']:item for item in record.get('assertions',[])}
    statuses = [assertions.get(item['id'],{}).get('status',UNVERIFIED)
                for item in case['assertions'] if item.get('mandatory',True)]
    if record.get('status')!='rendered':
        statuses.append(BLOCKED)
    return combine(statuses)


def evaluate_requirements(bundle: dict, cases: dict) -> dict:
    by_id = {record.get('case_id',identity):record for identity,record in cases.items()}
    results = {}
    pending = list(bundle['requirements'])
    while pending:
        next_pending = []
        for identity in pending:
            requirement = bundle['requirements'][identity]
            dependencies = requirement['dependencies']
            if any(dependency not in results for dependency in dependencies):
                next_pending.append(identity)
                continue
            if requirement['applicability']['status']=='excluded':
                results[identity] = {'status':NOT_APPLICABLE,'assertions':[],'dependencies':dependencies}
                continue
            evidence = []
            for reference in requirement.get('verification',[]):
                record = by_id.get(reference['case_id'],{})
                assertion = next((item for item in record.get('assertions',[]) if item['id']==reference['assertion_id']),None)
                status = assertion['status'] if assertion else UNVERIFIED
                if record.get('status','rendered')!='rendered' and status!=FAIL:
                    status = BLOCKED
                evidence.append({**reference,'status':status})
            statuses = [item['status'] for item in evidence]
            if not evidence or requirement.get('review_state')!='reviewed':
                statuses.append(UNVERIFIED)
            for dependency in dependencies:
                status = results[dependency]['status']
                statuses.append(FAIL if status==FAIL else PASS if status==PASS else BLOCKED)
            results[identity] = {'status':combine(statuses),'assertions':evidence,'dependencies':dependencies}
        if len(next_pending)==len(pending):
            raise ValueError('Requirement dependency graph is cyclic or has missing nodes')
        pending = next_pending
    for identity,result in results.items():
        requirement = bundle['requirements'][identity]
        result.update(source_id=requirement['source_id'],source_url=bundle['sources'][requirement['source_id']]['url'],sections=requirement['sections'],case_ids=requirement['case_ids'])
    return results
