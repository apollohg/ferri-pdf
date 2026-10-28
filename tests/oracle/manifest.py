"""Load the oracle's pinned sources, requirements, cases and benchmark contract."""
import hashlib
from collections import Counter
import json
import math
from pathlib import Path
import re

SCHEMA_VERSION = 1
SCOPES = ("sources", "inventory", "cases", "full")
IMPLEMENTATION_STATES = {"present", "partial", "absent", "unknown"}
APPLICABILITY_STATES = {"applicable", "excluded"}
DISPOSITIONS = {"requirements", "informative", "superseded", "excluded"}
EXCLUSION_CLASSES = {"interactive-only", "outside-conformance-class", "non-normative", "superseded"}
SOURCE_PIN = re.compile(r"/TR/\d{4}/[^/]*\d{8}(?:/|$)|/commit-snapshots/[0-9a-f]{40}/|/(?:blob|raw|commit)/[0-9a-f]{40}/|/reports/tr\d+/tr\d+-\d+\.html|/versions/Unicode\d+\.\d+\.\d+/|/rfc/rfc\d+(?:\.html|/|$)|githubusercontent\.com/[^/]+/[^/]+/[0-9a-f]{40}/")
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
REVISION = re.compile(r"[0-9a-f]{40}\Z")
ID = re.compile(r"[a-z][a-z0-9]*(?:[.-][a-z0-9]+)*\Z")


class ManifestError(ValueError):
    def __init__(self, file, pointer, reason):
        self.file = Path(file)
        self.pointer = pointer
        self.reason = reason
        super().__init__(f"{self.file}:{pointer}: {reason}")


def _read(path):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ManifestError(path, "/", f"duplicate JSON key {key!r}")
            result[key] = value
        return result
    try:
        value = json.loads(path.read_text(), object_pairs_hook=pairs)
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ManifestError(path, "/", str(error)) from error
    if not isinstance(value, dict) or type(value.get("schema_version")) is not int or value.get("schema_version") != SCHEMA_VERSION:
        raise ManifestError(path, "/schema_version", "unsupported or absent schema version")
    def finite(item,pointer):
        if isinstance(item,float) and not math.isfinite(item):
            raise ManifestError(path,pointer,'nonfinite JSON number')
        if isinstance(item,dict):
            for key,child in item.items():finite(child,pointer+'/'+key)
        elif isinstance(item,list):
            for index,child in enumerate(item):finite(child,pointer+'/'+str(index))
    finite(value,'')
    return value


def _collection(path, key, origins):
    values = _read(path).get(key)
    if not isinstance(values, list):
        raise ManifestError(path, f"/{key}", "expected a list")
    result = {}
    for index, record in enumerate(values):
        pointer = f"/{key}/{index}"
        if not isinstance(record, dict) or not ID.fullmatch(str(record.get("id", ""))):
            raise ManifestError(path, pointer, "missing or invalid stable id")
        identity = record["id"]
        if identity in result:
            raise ManifestError(path, pointer, f"duplicate id {identity!r}")
        result[identity] = record
        origins[(key, identity)] = (path, pointer)
    return result


def load_bundle(root: Path, *, scope: str = "full") -> dict:
    root = Path(root).resolve()
    if scope not in SCOPES:
        raise ManifestError(root, "/scope", f"unknown scope {scope!r}")
    origins = {}
    bundle = {key: {} for key in ("sources", "sections", "requirements", "cases", "benchmarks", "defaults")}
    bundle.update(root=root, scope=scope, _origins=origins)
    bundle["sources"] = _collection(root / "sources.json", "sources", origins)
    for source_id, source in bundle["sources"].items():
        path = root / "sections" / f"{source_id}.json"
        document = _read(path)
        if document.get("source_id") != source_id:
            raise ManifestError(path, "/source_id", "index belongs to a different source")
        if source.get("index_sha256") != hashlib.sha256(path.read_bytes()).hexdigest():
            raise ManifestError(path, "/", "section index hash differs from source lock")
        bundle["sections"][source_id] = document.get("sections")
    if SCOPES.index(scope) >= SCOPES.index("inventory"):
        files = sorted((root / "requirements").glob("*.json"))
        if not files:
            raise ManifestError(root / "requirements", "/", "no requirement files")
        for path in files:
            for identity, record in _collection(path, "requirements", origins).items():
                if identity in bundle["requirements"]:
                    raise ManifestError(path, "/requirements", f"duplicate id {identity!r}")
                bundle["requirements"][identity] = record
    if SCOPES.index(scope) >= SCOPES.index("cases"):
        bundle["cases"] = _collection(root / "cases.json", "cases", origins)
        bundle["defaults"] = _read(root / "defaults.json")
    if scope == "full":
        bundle["benchmarks"] = _read(root / "benchmarks.json")
    validate_bundle(bundle)
    if scope != "sources" and (root / "foundations.json").exists():
        from foundations import load_foundations
        bundle["foundations"] = load_foundations(bundle)
    return bundle


def _error(bundle, collection, identity, field, reason):
    file, pointer = bundle.get("_origins", {}).get(
        (collection, identity),
        (Path(bundle["root"]) / f"{collection}.json", f"/{collection}/{identity}"),
    )
    raise ManifestError(file, f"{pointer}/{field}", reason)


def _references(bundle, collection, identity, field, targets):
    record = bundle[collection][identity]
    references = record.get(field)
    if not isinstance(references, list) or any(not isinstance(ref, str) for ref in references):
        _error(bundle, collection, identity, field, "expected a list of reference IDs")
    for reference in references:
        if reference not in targets:
            _error(bundle, collection, identity, field, f"unknown reference {reference!r}")
    return references


def _acyclic(bundle, collection):
    records = bundle[collection]
    done = set()
    for identity in records:
        active = set()
        stack = [(identity, False)]
        while stack:
            current, leaving = stack.pop()
            if leaving:
                active.remove(current)
                done.add(current)
            elif current not in done:
                if current in active:
                    _error(bundle, collection, current, "dependencies", "dependency cycle")
                active.add(current)
                stack.append((current, True))
                stack.extend((child, False) for child in reversed(records[current].get("dependencies", [])))


def _number(value, *, positive=False):
    return not isinstance(value,bool) and isinstance(value,(int,float)) and math.isfinite(value) and (not positive or value>0)


def _assertion_shape(assertion,fail):
    kind = assertion.get('kind')
    if 'expected' not in assertion:
        fail('assertion needs an independent expected outcome')
    expected = assertion['expected']
    if 'basis' in assertion and not isinstance(assertion['basis'],dict):
        fail('assertion basis must be an object')
    if kind=='page-count':
        if type(expected) is not int or expected<1:fail('page count must be a positive integer')
        return
    if kind=='page-boxes':
        boxes = expected if isinstance(expected,list) else [expected]
        if not boxes or any(not isinstance(box,dict) or any(not _number(box.get(key),positive=True) for key in ('width_pt','height_pt')) for box in boxes):
            fail('page boxes need finite positive width_pt and height_pt')
        return
    if kind not in ('unicode-text','word-counts','image-placements','glyphs','fonts','region-ink','pdf-reference','raster-reference'):
        return
    if not isinstance(expected,dict):fail('expected assertion fields must be an object')
    if kind=='word-counts':
        counts=expected.get('minimum_counts')
        if not isinstance(counts,dict) or not counts or any(not isinstance(word,str) or not word or type(count) is not int or count<1 for word,count in counts.items()):fail('word inventory needs nonempty strings and positive counts')
    elif kind=='image-placements':
        hashes=expected.get('raster_pixels_sha256')
        if not isinstance(hashes,list) or not hashes or len(hashes)%2 or any(not SHA256.fullmatch(str(value)) for value in hashes):fail('image inventory needs paired source pixel hashes')
        size=expected.get('size_pt')
        if not isinstance(size,list) or len(size)!=2 or any(not _number(value,positive=True) for value in size):fail('image inventory needs finite positive placement dimensions')
        samples=expected.get('vector_samples')
        if not isinstance(samples,list) or not samples:fail('vector paint needs independent colour samples')
        for sample in samples:
            if not isinstance(sample,dict) or not isinstance(sample.get('offset_pt'),list) or len(sample['offset_pt'])!=2 or any(not _number(value) for value in sample['offset_pt']) or not isinstance(sample.get('rgb'),list) or len(sample['rgb'])!=3 or any(type(value) is not int or not 0<=value<=255 for value in sample['rgb']):fail('vector paint samples need coordinate pairs and RGB channels')
    elif kind=='unicode-text':
        if 'per_page' in expected:
            if not isinstance(expected['per_page'],list) or not expected['per_page']:fail('per_page needs nonempty text contracts')
            for item in expected['per_page']:_assertion_shape({'kind':kind,'expected':item},fail)
            return
        texts = expected.get('patterns',expected.get('texts',[expected.get('text')]))
        if not isinstance(texts,list) or not texts or any(not isinstance(text,str) or not text for text in texts):fail('text expectations need nonempty strings')
        if 'patterns' in expected:
            for pattern in texts:
                try:re.compile(pattern)
                except re.error as error:fail(f'invalid text pattern: {error}')
        pages = expected.get('pages','all')
        if pages not in ('all','any') and (not isinstance(pages,list) or not pages or any(type(page) is not int or page<1 for page in pages)):fail('text pages need all, any, or positive page numbers')
        if 'count_per_page' in expected and (type(expected['count_per_page']) is not int or expected['count_per_page']<0):fail('text count must be a nonnegative integer')
    elif kind=='glyphs':
        if 'codepoint_ranges' in expected:
            ranges=expected['codepoint_ranges']
            if not isinstance(ranges,list) or not ranges or any(not isinstance(pair,list) or len(pair)!=2 or any(type(value) is not int or not 0<=value<=0x10ffff for value in pair) or pair[0]>pair[1] for pair in ranges):fail('glyph selectors need ordered Unicode ranges')
        if 'minimum_glyphs' in expected and (type(expected['minimum_glyphs']) is not int or expected['minimum_glyphs']<1):fail('glyph coverage must be nonvacuous')
        if not expected or not set(expected)&{'text','font','render_mode','fill','origins_pt'}:fail('glyph assertion needs an observable constraint')
        if 'origins_pt' in expected and (not isinstance(expected['origins_pt'],list) or any(not isinstance(point,list) or len(point)!=2 or not all(_number(value) for value in point) for point in expected['origins_pt'])):fail('glyph origins need finite coordinate pairs')
        if 'fill' in expected and (not isinstance(expected['fill'],list) or len(expected['fill'])!=4 or not all(_number(value) and 0<=value<=1 for value in expected['fill'])):fail('glyph fill needs RGBA values in [0,1]')
    elif kind=='fonts':
        if not isinstance(expected.get('names'),list) or not expected['names'] or any(not isinstance(name,str) or not name for name in expected['names']):fail('font assertion needs explicit face names')
        if 'embedded' in expected and type(expected['embedded']) is not bool:fail('embedded must be a boolean')
    elif kind=='region-ink':
        rectangle = expected.get('rectangle_pt')
        if type(expected.get('page')) is not int or expected['page']<1 or not isinstance(rectangle,list) or len(rectangle)!=4 or not all(_number(value) for value in rectangle) or rectangle[2]<=rectangle[0] or rectangle[3]<=rectangle[1]:fail('ink region needs a page and finite ordered rectangle')
        bounds = [expected[key] for key in ('minimum_dark_pixels','maximum_dark_pixels') if key in expected]
        if not bounds or any(not _number(value) or value<0 for value in bounds):fail('ink region needs a nonnegative finite coverage bound')
    elif not isinstance(expected.get('path'),str) or not SHA256.fullmatch(str(expected.get('sha256',''))):
        fail('reference needs a local path and SHA-256')


def _validate_tolerances(path,profile):
    def fail(field,reason):raise ManifestError(path,'/tolerances/'+field,reason)
    if not isinstance(profile,dict):fail('','expected tolerance object')
    positive = {'resolution_dpi','points_per_inch','glyph_truncation_units_per_em','line_cluster_pt','mismatch_report_limit','command_timeout_s'}
    nonnegative = {'glyph_origin_pt','glyph_matrix_pt','word_box_slack_pt','text_ink_floor_px','direction_digits'}
    fractions = {'glyph_colour','changed_ink_fraction','regional_ink_coverage_fraction'}
    channels = {'pixel_delta','regional_dark_threshold','ink_lightness_cutoff'}
    integers = {'resolution_dpi','glyph_truncation_units_per_em','mismatch_report_limit','text_ink_floor_px','direction_digits'}|channels
    nested = {'word_box_em','metric_compatible_fonts'}
    required = positive|nonnegative|fractions|channels|nested
    for field in sorted(required):
        value = profile.get(field)
        if field in nested:continue
        if not _number(value) or (field in integers and type(value) is not int):fail(field,'expected a finite '+('integer' if field in integers else 'number'))
        if field in positive and value<=0 or field in nonnegative and value<0:fail(field,'out of range')
        if field in fractions and not 0<=value<=1 or field in channels and not 0<=value<=255:fail(field,'out of range')
    if set(profile)-required:fail('','unknown tolerance fields: '+str(sorted(set(profile)-required)))
    word_box=profile.get('word_box_em')
    if not isinstance(word_box,dict) or set(word_box)!={'before','after','below','above'}:fail('word_box_em','expected complete coordinate object')
    if any(not _number(item) or item<0 for item in word_box.values()):fail('word_box_em','expected finite nonnegative coordinates')
    fonts=profile.get('metric_compatible_fonts')
    if not isinstance(fonts,dict) or any(not isinstance(key,str) or not key or not isinstance(value,str) or not value for key,value in fonts.items()):fail('metric_compatible_fonts','expected font-name string mapping')


def _validate_shapes(bundle):
    defaults = bundle.get('defaults',{})
    if not isinstance(defaults,dict):
        raise ManifestError(Path(bundle['root'])/'defaults.json','/','expected defaults object')
    if bundle.get('scope') in ('cases','full'):
        _validate_tolerances(Path(bundle['root'])/'defaults.json',defaults.get('tolerances'))
    for identity,source in bundle['sources'].items():
        if not isinstance(source.get('url'),str):_error(bundle,'sources',identity,'url','expected an immutable URL string')
        references = source.get('reference_dispositions',[])
        if not isinstance(references,list) or any(not isinstance(item,dict) for item in references):_error(bundle,'sources',identity,'reference_dispositions','expected reference objects')
    for source_id,sections in bundle['sections'].items():
        if isinstance(sections,list):
            for section in sections:
                if isinstance(section,dict) and (not isinstance(section.get('id'),str) or not isinstance(section.get('title'),str) or not isinstance(section.get('disposition',{}),dict) or not isinstance(section.get('disposition',{}).get('kind',''),str)):
                    raise ManifestError(Path(bundle['root'])/'sections'/f'{source_id}.json','/sections','section IDs must be strings and dispositions objects')
    for identity,requirement in bundle['requirements'].items():
        for field in ('applicability','implementation'):
            if not isinstance(requirement.get(field),dict):_error(bundle,'requirements',identity,field,'expected an object')
        if not isinstance(requirement.get('source_id'),str):_error(bundle,'requirements',identity,'source_id','expected source ID string')
        if not isinstance(requirement.get('verification',[]),list):_error(bundle,'requirements',identity,'verification','expected case/assertion evidence list')
        if not isinstance(requirement['applicability'].get('status'),str) or not isinstance(requirement['implementation'].get('status'),str):_error(bundle,'requirements',identity,'status','expected status strings')
        if requirement['applicability'].get('status')=='excluded' and not isinstance(requirement['applicability'].get('basis'),dict):_error(bundle,'requirements',identity,'applicability','excluded requirements need a source-backed basis object')
        for reference in requirement.get('verification',[]):
            if not isinstance(reference,dict) or not isinstance(reference.get('case_id'),str) or not isinstance(reference.get('assertion_id'),str):_error(bundle,'requirements',identity,'verification','expected case/assertion ID objects')
            if bundle.get('scope') in ('cases','full'):
                case = bundle['cases'].get(reference['case_id'],{})
                if not any(isinstance(item,dict) and item.get('id')==reference['assertion_id'] for item in case.get('assertions',[])):_error(bundle,'requirements',identity,'verification','unknown case/assertion evidence reference')
    for identity,case in bundle['cases'].items():
        if case.get('input') is not None and not isinstance(case['input'],str):_error(bundle,'cases',identity,'input','expected local input path string')
        if not isinstance(case.get('provenance',{}),dict):_error(bundle,'cases',identity,'provenance','expected provenance object')
        resources = case.get('resources',[])
        if not isinstance(resources,list):_error(bundle,'cases',identity,'resources','expected a resource list')
        for resource in resources:
            if not isinstance(resource,dict) or not isinstance(resource.get('path'),str) or Path(resource['path']).is_absolute() or '..' in Path(resource['path']).parts or not SHA256.fullmatch(str(resource.get('sha256',''))):_error(bundle,'cases',identity,'resources','resource needs a safe relative path and SHA-256')
        if not isinstance(case.get('warnings',[]),list):_error(bundle,'cases',identity,'warnings','expected warning dispositions list')
        for warning in case.get('warnings',[]):
            if not isinstance(warning,dict) or not isinstance(warning.get('pattern'),str) or warning.get('status') not in ('accepted','blocked') or not warning.get('reason'):_error(bundle,'cases',identity,'warnings','warnings need pattern, disposition status and rationale')
            try:re.compile(warning['pattern'])
            except re.error as error:_error(bundle,'cases',identity,'warnings',str(error))
        for assertion in case.get('assertions',[]) if isinstance(case.get('assertions'),list) else []:
            if isinstance(assertion,dict):
                if not isinstance(assertion.get('id'),str) or not isinstance(assertion.get('kind'),str):_error(bundle,'cases',identity,'assertions','assertion IDs and kinds must be strings')
                _assertion_shape(assertion,lambda reason:_error(bundle,'cases',identity,'assertions',reason))
    groups = bundle.get('benchmarks',{}).get('classes',[])
    if not isinstance(groups,list) or any(not isinstance(group,dict) or not isinstance(group.get('id'),str) or not isinstance(group.get('case_ids'),list) or any(not isinstance(identity,str) for identity in group['case_ids']) for group in groups):
        raise ManifestError(Path(bundle['root'])/'benchmarks.json','/classes','expected class objects with case ID lists')


def _validate_controls(bundle):
    def exists(reference):
        return isinstance(reference,dict) and isinstance(reference.get('source_id'),str) and isinstance(reference.get('section_id'),str) and any(item['id']==reference['section_id'] for item in bundle['sections'].get(reference['source_id'],[]))
    def proof(basis):
        return exists(basis) and basis.get('sha256')==bundle['sources'][basis['source_id']]['sha256']
    for source_id,source in bundle['sources'].items():
        overlaps=source.get('overlap_dispositions',[])
        if not isinstance(overlaps,list):_error(bundle,'sources',source_id,'overlap_dispositions','expected explicit overlap review records')
        for record in overlaps:
            if not isinstance(record,dict) or record.get('status')!='unresolved' or not isinstance(record.get('other_source_id'),str) or record.get('other_source_id') not in bundle['sources'] or record.get('other_source_id')==source_id or not record.get('reason'):
                _error(bundle,'sources',source_id,'overlap_dispositions','unresolved overlap needs another pinned source and rationale; resolved sections use controlling references')
        for section in bundle['sections'][source_id]:
            path=Path(bundle['root'])/'sections'/f'{source_id}.json'
            controls=section.get('controlling_definitions',[])
            if not isinstance(controls,list):raise ManifestError(path,'/controlling_definitions','expected definition-level controllers')
            for control in controls:
                if not isinstance(control,dict) or not control.get('behaviour') or not control.get('reason') or not exists(control.get('controlling')) or not proof(control.get('basis')):
                    raise ManifestError(path,'/controlling_definitions','controller needs behaviour, rationale, target and pinned source proof')
            disposition=section.get('disposition',{})
            if disposition.get('kind')=='superseded' and 'basis' in disposition and not proof(disposition['basis']):
                raise ManifestError(path,'/disposition/basis','replacement proof differs from pinned source')


def validate_bundle(bundle: dict) -> None:
    _validate_shapes(bundle)
    scope = bundle.get("scope", "full")
    if scope not in SCOPES:
        raise ManifestError(bundle["root"], "/scope", "unknown validation scope")
    for identity, source in bundle["sources"].items():
        if not SHA256.fullmatch(str(source.get("sha256", ""))):
            _error(bundle, "sources", identity, "sha256", "source needs a SHA-256 content pin")
        if not SOURCE_PIN.search(source.get("url", "")) and not REVISION.fullmatch(str(source.get("revision", ""))):
            _error(bundle, "sources", identity, "url", "source URL is not immutable and has no fixed revision")
        if "rendered" in source:
            rendered = source["rendered"]
            if not isinstance(rendered, dict) or not isinstance(rendered.get("url"), str) or not rendered["url"]:
                _error(bundle, "sources", identity, "rendered", "rendered source needs a provenance URL")
            if not SHA256.fullmatch(str(rendered.get("sha256", ""))):
                _error(bundle, "sources", identity, "rendered/sha256", "rendered source needs a content hash")
            if not REVISION.fullmatch(str(source.get("revision", ""))) or rendered.get("revision") != source["revision"]:
                _error(bundle, "sources", identity, "rendered/revision", "rendered revision differs from primary source")
        for field in ("licence", "maturity"):
            if not source.get(field):
                _error(bundle, "sources", identity, field, "required source provenance is missing")
        _references(bundle, "sources", identity, "dependencies", bundle["sources"])
    section_ids = {}
    for source_id in bundle["sources"]:
        sections = bundle["sections"].get(source_id)
        path = Path(bundle["root"]) / "sections" / f"{source_id}.json"
        if not isinstance(sections, list) or not sections:
            raise ManifestError(path, "/sections", "source needs a nonempty section index")
        seen = set()
        for index, section in enumerate(sections):
            pointer = f"/sections/{index}"
            if not isinstance(section, dict) or not section.get("id") or not section.get("title"):
                raise ManifestError(path, pointer, "section needs an ID and title")
            if section["id"] in seen:
                raise ManifestError(path, pointer, "duplicate section ID")
            seen.add(section["id"])
            if scope == "sources":
                continue
            disposition = section.get("disposition", {})
            if disposition.get("kind") not in DISPOSITIONS:
                raise ManifestError(path, pointer, "section has no valid disposition")
            if disposition["kind"] == "requirements":
                references = disposition.get("requirement_ids", [])
                if not references or any(ref not in bundle["requirements"] for ref in references):
                    raise ManifestError(path, pointer, "section needs existing requirement IDs")
                for reference in references:
                    requirement = bundle['requirements'][reference]
                    if requirement.get('source_id') != source_id or section['id'] not in requirement.get('sections', []):
                        raise ManifestError(path, pointer, 'requirement does not describe this source section')
            elif not disposition.get("reason"):
                raise ManifestError(path, pointer, "disposition needs a source-backed reason")
            if disposition['kind'] == 'superseded':
                controlling = disposition.get('controlling', {})
                target_source = controlling.get('source_id')
                target_section = controlling.get('section_id')
                targets = bundle['sections'].get(target_source, [])
                if not any(item.get('id') == target_section for item in targets) or (target_source, target_section) == (source_id, section['id']):
                    raise ManifestError(path, pointer, 'invalid controlling section for superseded material')
        section_ids[source_id] = seen
    _validate_controls(bundle)
    if scope == "sources":
        return
    for identity, requirement in bundle["requirements"].items():
        source_id = requirement.get("source_id")
        if source_id not in bundle["sources"]:
            _error(bundle, "requirements", identity, "source_id", "unknown source")
        _references(bundle, "requirements", identity, "sections", section_ids[source_id])
        if not requirement["sections"] or not requirement.get("behaviour"):
            _error(bundle, "requirements", identity, "behaviour", "requirement needs behaviour and source sections")
        applicability = requirement.get("applicability", {})
        if applicability.get("status") not in APPLICABILITY_STATES or not applicability.get("reason"):
            _error(bundle, "requirements", identity, "applicability", "missing applicability status or rationale")
        if applicability['status'] == 'excluded':
            basis = applicability.get('basis', {})
            if basis.get('classification') not in EXCLUSION_CLASSES or basis.get('source_id') not in section_ids or basis.get('section_id') not in section_ids.get(basis.get('source_id'), set()):
                _error(bundle, 'requirements', identity, 'applicability', 'exclusion needs a valid source-backed profile classification')
        if requirement.get("implementation", {}).get("status") not in IMPLEMENTATION_STATES:
            _error(bundle, "requirements", identity, "implementation", "unknown implementation state")
        _references(bundle, "requirements", identity, "dependencies", bundle["requirements"])
        if scope in ("cases", "full"):
            _references(bundle, "requirements", identity, "case_ids", bundle["cases"])
    _acyclic(bundle, "requirements")
    if scope == "inventory":
        return
    for identity, case in bundle["cases"].items():
        _references(bundle, "cases", identity, "requirement_ids", bundle["requirements"])
        timeout = case.get("timeout_s")
        if isinstance(timeout, bool) or not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or timeout <= 0:
            _error(bundle, "cases", identity, "timeout_s", "timeout must be positive and finite")
        if not case.get("input") and not case.get("generator_id"):
            _error(bundle, "cases", identity, "input", "case needs input or generator identity")
        if not isinstance(case.get("assertions"), list):
            _error(bundle, "cases", identity, "assertions", "expected assertion list")
        seen = set()
        for assertion in case["assertions"]:
            if not isinstance(assertion, dict) or not assertion.get("id") or not assertion.get("kind"):
                _error(bundle, "cases", identity, "assertions", "assertion needs ID and kind")
            if assertion["id"] in seen:
                _error(bundle, "cases", identity, "assertions", "duplicate assertion ID")
            seen.add(assertion["id"])
    if scope == "full":
        classes = bundle["benchmarks"].get("classes")
        if not isinstance(classes, list) or not classes:
            raise ManifestError(Path(bundle["root"]) / "benchmarks.json", "/classes", "suite needs classes")
        seen_classes, seen_cases = set(), set()
        for index, group in enumerate(classes):
            pointer = f"/classes/{index}"
            if group.get("id") in seen_classes or not group.get("id"):
                raise ManifestError(Path(bundle["root"]) / "benchmarks.json", pointer, "missing or duplicate class ID")
            seen_classes.add(group["id"])
            for case_id in group.get("case_ids", []):
                if case_id not in bundle["cases"] or case_id in seen_cases:
                    raise ManifestError(Path(bundle["root"]) / "benchmarks.json", pointer, "unknown or repeated suite case")
                seen_cases.add(case_id)


def suite_case_ids(bundle):
    return [identity for group in bundle['benchmarks']['classes'] for identity in group['case_ids']]


def select_cases(bundle: dict, case_ids: list[str] | None) -> list[dict]:
    if case_ids is None:
        return list(bundle["cases"].values())
    unknown = set(case_ids) - bundle["cases"].keys()
    if unknown:
        raise ManifestError(Path(bundle["root"]) / "cases.json", "/selection", f"unknown case IDs: {sorted(unknown)}")
    return [bundle["cases"][identity] for identity in dict.fromkeys(case_ids)]


def inventory_summary(bundle: dict) -> dict:
    requirements = bundle['requirements']
    applicable = {identity: record for identity, record in requirements.items() if record['applicability']['status'] == 'applicable'}
    unresolved = []
    for source_id, source in bundle['sources'].items():
        for reference in source.get('reference_dispositions', []):
            if reference.get('status') == 'unresolved':
                unresolved.append({'source_id': source_id, **reference})
    unresolved_overlaps = [{'source_id':source_id,**record} for source_id,source in bundle['sources'].items() for record in source.get('overlap_dispositions',[]) if record.get('status')=='unresolved']
    missing_evidence = []
    for identity, requirement in applicable.items():
        cases = [bundle['cases'][case_id] for case_id in requirement.get('case_ids', []) if case_id in bundle['cases']]
        if not cases or not any(assertion.get('evidence') for case in cases for assertion in case.get('assertions', [])):
            missing_evidence.append(identity)
    unmapped = [{'source_id': source_id, 'section_id': section['id']} for source_id, sections in bundle['sections'].items() for section in sections if not section.get('disposition')]
    return {
        'requirements_total': len(requirements), 'applicable': len(applicable),
        'excluded': len(requirements) - len(applicable), 'verified': 0,
        'verification_basis': 'Inventory only; execution evidence is evaluated separately.',
        'implementation': dict(Counter(record['implementation']['status'] for record in applicable.values())),
        'families': dict(Counter(record.get('family', identity.split('.')[0]) for identity, record in applicable.items())),
        'missing_evidence_ids': sorted(missing_evidence),
        'unresolved_imports': unresolved, 'unmapped_sections': unmapped,
        'source_closure_complete': not unresolved,
        'inventory_complete': not unmapped and not unresolved and not unresolved_overlaps,
        'unresolved_overlaps': unresolved_overlaps,
    }
