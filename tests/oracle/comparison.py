"""Shared PDF comparisons with explicit settings and output ownership."""
import bisect
import glob
import copy
from functools import lru_cache
import itertools
import json
import math
from pathlib import Path
import shutil
import unicodedata
import xml.etree.ElementTree as ET
import zlib
from PIL import Image, ImageChops, ImageDraw, ImageStat
import pdf_content
import bidi_text
from execution import DEFAULT_COMMAND_TIMEOUT_S, checked_command, digest

HERE=Path(__file__).resolve().parent
NS={"x":"http://www.w3.org/1999/xhtml"}
CHANNEL_MAX=255
REQUIRED_CALIBRATION_CHECKS = (
    'identical_pass','two_point_glyph_shift_rejected','missing_glyph_rejected','missing_page_rejected',
    'identical_pixels_pass','two_point_pixel_shift_rejected','colour_only_difference_rejected',
)


def calibration_valid(result):
    return all(result.get(check) is True for check in REQUIRED_CALIBRATION_CHECKS)


@lru_cache(maxsize=1)
def _defaults():
    return json.loads((HERE/"manifests/defaults.json").read_text())

def default_settings() -> dict:
    return copy.deepcopy(_defaults()["tolerances"])

def canonical(text):
    return unicodedata.normalize('NFKC', ''.join((c for c in text if unicodedata.category(c) != 'Cf')))

def phrase_words(text, glyphs):
    """Corroborate a replacement phrase, then count its painted visual words."""
    words, word = [], ''
    for _, glyph_text, word_start in sorted(glyphs):
        if word_start and word:
            words.append(word)
            word = ''
        word += glyph_text
    if word:
        words.append(word)
    if sorted(''.join(words)) != sorted(text):
        return []
    return words

def extract(pdf, lines, *, replacement_text=(), glyphs=(), settings=None, output_dir=None, timeout_s=DEFAULT_COMMAND_TIMEOUT_S):
    """pdftotext words per visual line, concatenated in writing order without whitespace.

    Each word joins the glyph line (`glyph_lines`) whose painted origins fall inside its box and spell it, so lines follow
    content-stream baselines rather than pdftotext top edges, which depend on font descriptor heights. pdftotext still
    supplies the text. Diagonal text is dropped (`-nodiag`); `rotated_lines` compares it along its writing direction."""
    if settings is None:
        settings = default_settings()
    output_dir = Path(output_dir) if output_dir is not None else Path(pdf).parent
    output_dir.mkdir(parents=True, exist_ok=True)
    xml = output_dir / (pdf.stem + '-' + digest(pdf)[:12] + '.xml')
    checked_command(['pdftotext', '-nodiag', '-bbox-layout', pdf, xml], output_dir=output_dir / 'logs', timeout_s=timeout_s)
    result = []
    for number, page in enumerate(ET.parse(xml).findall('.//x:page', NS), start=1):
        height = float(page.attrib['height'])
        origins = sorted(((height - glyph['glyph'].origin[1], glyph['glyph'].origin[0], index, glyph['along'], canonical(glyph['glyph'].text), glyph['word_start']) for index, line in enumerate(lines) if line[0]['glyph'].page == number for glyph in line))
        tops = [origin[0] for origin in origins]
        grouped, unmatched = ({}, [])
        words = []
        for word in page.findall('.//x:word', NS):
            left, top, right, bottom = (float(word.attrib[key]) for key in ('xMin', 'yMin', 'xMax', 'yMax'))
            tokens = canonical(word.text or '').split()
            text = ''.join(tokens)
            record = {'text':text,'rectangle_pt':[left,top,right,bottom]}
            if len(tokens) > 1:
                record['visual_words'] = []
            words.append(record)
            candidates = {}
            for _, x, index, along, glyph_text, word_start in origins[bisect.bisect_left(tops, top - settings['word_box_slack_pt']):bisect.bisect_right(tops, bottom + settings['word_box_slack_pt'])]:
                if left - settings['word_box_slack_pt'] <= x <= right + settings['word_box_slack_pt']:
                    candidates.setdefault(index, []).append((along, glyph_text, word_start))
            if not candidates:
                unmatched.append({'top_pt': top, 'left_pt': left, 'text': text})
                continue
            index = max(candidates, key=lambda line: (''.join((glyph_text for _, glyph_text, _ in sorted(candidates[line]))) == text, len(candidates[line])))
            if len(tokens) > 1:
                record['visual_words'] = phrase_words(text, candidates[index])
            grouped.setdefault(index, []).append((min((along for along, _, _ in candidates[index])), text))
        page_lines = [{'origin_pt': [round(lines[index][0]['glyph'].origin[0], 2), round(height - lines[index][0]['glyph'].origin[1], 2)], 'text': ''.join((text for _, text in sorted(words)))} for index, words in sorted(grouped.items())]
        if unmatched:
            page_lines.append({'origin_pt': None, 'text': ''.join((word['text'] for word in sorted(unmatched, key=lambda word: (word['top_pt'], word['left_pt']))))})
        result.append({'width': float(page.attrib['width']), 'height': height, 'lines': page_lines,'words':words})
    corroborate_replacement_text(result, lines, glyphs, replacement_text)
    return result


def corroborate_replacement_text(pages, lines, glyphs, spans):
    def compact(text):
        return ''.join(canonical(text).split())

    membership = {id(item['glyph']): index for index, line in enumerate(lines) for item in line}
    for span in spans:
        painted = glyphs[span.glyph_start:span.glyph_end]
        indices = {membership[id(glyph)] for glyph in painted if id(glyph) in membership}
        owned = {id(glyph) for glyph in painted}
        visual = ''.join(item['glyph'].text for index in sorted(indices) for item in lines[index]
                         if id(item['glyph']) in owned)
        logical = compact(span.text)
        rtl = any(unicodedata.bidirectional(char) in ('R', 'AL', 'RLE', 'RLO', 'RLI') for char in span.text)
        orders = bidi_text.visual_orders(span.text) if rtl else {span.text}
        page = pages[span.page - 1]
        if compact(visual) not in {compact(order) for order in orders}:
            page.setdefault('replacement_text_errors', []).append({'logical': span.text, 'painted': visual})
            continue
        # Only a whole visual line has sufficient context to replace Poppler's exported order.
        if len(indices) != 1:
            continue
        index = next(iter(indices))
        line = lines[index]
        if any(id(item['glyph']) not in owned for item in line):
            continue
        origin = [round(line[0]['glyph'].origin[0], 2), round(page['height'] - line[0]['glyph'].origin[1], 2)]
        matching = [item for item in page['lines'] if item['origin_pt'] == origin]
        if len(matching) == 1 and matching[0]['text'] in (logical, compact(visual)):
            matching[0]['text'] = compact(visual)


def text_lines(reference, candidate):
    page_match = len(reference) == len(candidate)
    differences = []
    for index, (a, b) in enumerate(zip(reference, candidate), start=1):
        page_match &= a['width'] == b['width'] and a['height'] == b['height']
        if (a.get('replacement_text_errors') or b.get('replacement_text_errors')
                or [line['text'] for line in a['lines']] != [line['text'] for line in b['lines']]):
            differences.append({'page': index, 'reference_lines': a['lines'], 'candidate_lines': b['lines'],
                                'reference_replacement_errors': a.get('replacement_text_errors', []),
                                'candidate_replacement_errors': b.get('replacement_text_errors', [])})
    return {'page_match': page_match, 'text_line_match': page_match and (not differences), 'text_differences': differences}

def writing_direction(matrix, *, settings=None):
    if settings is None:
        settings = default_settings()
    return round(math.atan2(matrix[1], matrix[0]), settings['direction_digits'])

def glyph_lines(glyphs, *, settings=None):
    """Painted glyphs grouped into lines along their writing direction, in visual order.

    Whitespace glyphs paint nothing; they are dropped and only mark the next glyph as a word start."""
    if settings is None:
        settings = default_settings()
    keyed = []
    for glyph in glyphs:
        a, b = glyph.matrix[:2]
        size = math.hypot(a, b)
        ux, uy = (a / size, b / size)
        x, y = glyph.origin
        keyed.append(((glyph.page, writing_direction(glyph.matrix, settings=settings)), ux * y - uy * x, ux * x + uy * y, glyph))
    keyed.sort(key=lambda item: (item[0], -item[1]))
    clusters = []
    for direction, across, along, glyph in keyed:
        if not clusters or clusters[-1]['direction'] != direction or clusters[-1]['across'] - across > settings['line_cluster_pt']:
            clusters.append({'direction': direction, 'glyphs': []})
        clusters[-1]['across'] = across
        clusters[-1]['glyphs'].append((along, across, glyph))
    lines = []
    for cluster in clusters:
        line, word_start = ([], True)
        for along, across, glyph in sorted(cluster['glyphs'], key=lambda item: (item[0], -item[1])):
            if glyph.text and glyph.text.isspace():
                word_start = True
                continue
            line.append({'along': along, 'across': across, 'glyph': glyph, 'word_start': word_start})
            word_start = False
        if line:
            lines.append(line)
    return lines

def rotated_lines(lines, *, settings=None):
    """Text of each glyph line not written along +x, read from the content stream along its writing direction."""
    if settings is None:
        settings = default_settings()
    return [{'page': line[0]['glyph'].page, 'direction_rad': writing_direction(line[0]['glyph'].matrix, settings=settings), 'text': ''.join((canonical(glyph['glyph'].text) for glyph in line))} for line in lines if writing_direction(line[0]['glyph'].matrix, settings=settings)]

def compare_rotated_lines(reference, candidate, *, settings=None):
    if settings is None:
        settings = default_settings()
    differences = [{'reference': a, 'candidate': b} for a, b in itertools.zip_longest(reference, candidate) if a != b]
    return {'rotated_line_match': not differences, 'rotated_line_differences': differences[:settings['mismatch_report_limit']]}

def describe(glyph):
    return {'page': glyph.page, 'text': glyph.text, 'font': glyph.font, 'render_mode': glyph.render_mode, 'fill': glyph.fill, 'stroke': glyph.stroke, 'matrix': [round(value, 4) for value in glyph.matrix]}

def same_colour(a, b, *, settings=None):
    if settings is None:
        settings = default_settings()
    if a is None or b is None:
        return a is b
    return len(a) == len(b) and all((x == y if isinstance(x, str) or isinstance(y, str) else abs(x - y) <= settings['glyph_colour'] for x, y in zip(a, b)))

def glyph_problems(reference, candidate, advances, *, settings=None):
    """Differences between paired glyphs; `advances` counts glyphs since the current word start."""
    if settings is None:
        settings = default_settings()
    r, c = (reference['glyph'], candidate['glyph'])
    problems = []
    if canonical(r.text) != canonical(c.text):
        problems.append('text')
    if c.font not in (r.font, settings['metric_compatible_fonts'].get(r.font)):
        problems.append('font')
    if r.render_mode != c.render_mode:
        problems.append('render mode')
    if not same_colour(r.fill, c.fill, settings=settings) or not same_colour(r.stroke, c.stroke, settings=settings):
        problems.append('colour')
    if max((abs(x - y) for x, y in zip(r.matrix[:4], c.matrix[:4]))) > settings['glyph_matrix_pt']:
        problems.append('matrix')
    if abs(candidate['across'] - reference['across']) > settings['glyph_origin_pt']:
        problems.append('baseline')
    truncation = advances * math.hypot(*r.matrix[:2]) / settings['glyph_truncation_units_per_em']
    if not -settings['glyph_origin_pt'] <= candidate['along'] - reference['along'] <= settings['glyph_origin_pt'] + truncation:
        problems.append('origin')
    return problems

def glyph_check(reference, candidate, *, settings=None):
    """Pair painted glyphs line by line in visual order and compare identity, paint, matrix and origin."""
    if settings is None:
        settings = default_settings()
    mismatches, count, compared, along, across = ([], 0, 0, 0.0, 0.0)
    if [len(line) for line in reference] != [len(line) for line in candidate]:
        for index in range(max(len(reference), len(candidate))):
            a = reference[index] if index < len(reference) else []
            b = candidate[index] if index < len(candidate) else []
            if len(a) != len(b):
                mismatches.append({'kind': 'glyph-count', 'line': index, 'page': (a or b)[0]['glyph'].page, 'reference_glyphs': len(a), 'candidate_glyphs': len(b), 'reference': ''.join((g['glyph'].text for g in a)), 'candidate': ''.join((g['glyph'].text for g in b))})
                count = 1
                break
    else:
        for index, (a, b) in enumerate(zip(reference, candidate)):
            advances = 0
            for position, (r, c) in enumerate(zip(a, b)):
                advances = 0 if r['word_start'] or c['word_start'] else advances + 1
                compared += 1
                along = max(along, abs(c['along'] - r['along']))
                across = max(across, abs(c['across'] - r['across']))
                problems = glyph_problems(r, c, advances, settings=settings)
                if problems:
                    count += 1
                    if len(mismatches) < settings['mismatch_report_limit']:
                        mismatches.append({'kind': ', '.join(problems), 'line': index, 'glyph': position, 'advances_since_word_start': advances, 'origin_along_delta_pt': c['along'] - r['along'], 'reference': describe(r['glyph']), 'candidate': describe(c['glyph'])})
    return {'glyph_match': count == 0, 'glyphs_compared': compared, 'max_glyph_along_error_pt': along, 'max_glyph_across_error_pt': across, 'glyph_mismatch_count': count, 'glyph_mismatches': mismatches}

class AnalysisError(RuntimeError):
    pass

def analyse(pdf, timeout_s=DEFAULT_COMMAND_TIMEOUT_S, *, settings=None, output_dir=None):
    if settings is None:
        settings = default_settings()
    output_dir = Path(output_dir) if output_dir is not None else Path(pdf).parent
    output_dir.mkdir(parents=True, exist_ok=True)
    try:
        inspected = pdf_content.inspect(pdf)
        lines = glyph_lines(inspected['glyphs'], settings=settings)
        pages = extract(pdf, lines, replacement_text=inspected['replacement_text'], glyphs=inspected['glyphs'], settings=settings, output_dir=output_dir, timeout_s=timeout_s)
        for image in inspected['images']:
            a,b,c,d,e,f = image['matrix']
            points = [(e,f),(a+e,b+f),(c+e,d+f),(a+c+e,b+d+f)]
            height = pages[image['page']-1]['height']
            image['rectangle_pt'] = [min(x for x,y in points),height-max(y for x,y in points),max(x for x,y in points),height-min(y for x,y in points)]
        return {'pages':pages,'glyph_lines':lines,'rotated_lines':rotated_lines(lines,settings=settings),'images':inspected['images']}
    except (pdf_content.PdfError, ET.ParseError, ValueError, IndexError, KeyError, zlib.error) as error:
        raise AnalysisError(f'{pdf}: PDF analysis failed: {error}') from error

def text_free(pdf, *, output_dir=None):
    output_dir = Path(output_dir) if output_dir is not None else Path(pdf).parent
    output_dir.mkdir(parents=True, exist_ok=True)
    target = output_dir / (pdf.stem + '-' + digest(pdf)[:12] + '.text-free.pdf')
    pdf_content.write_text_free(pdf, target)
    if pdf_content.glyphs(target):
        raise AnalysisError(f'text-free copy of {pdf} still paints glyphs')
    return target

def raster(pdf, *, settings=None, output_dir=None, timeout_s=DEFAULT_COMMAND_TIMEOUT_S):
    if settings is None:
        settings = default_settings()
    output_dir = Path(output_dir) if output_dir is not None else Path(pdf).parent
    output_dir.mkdir(parents=True, exist_ok=True)
    prefix = output_dir / (pdf.stem + '-' + digest(pdf)[:12])
    for previous in output_dir.glob(glob.escape(prefix.name) + '-*.png'):
        previous.unlink()
    checked_command(['pdftoppm', '-r', settings['resolution_dpi'], '-png', pdf, prefix], output_dir=output_dir / 'logs', timeout_s=timeout_s)
    images = sorted(output_dir.glob(glob.escape(prefix.name) + '-*.png'), key=lambda p: int(p.stem.rsplit('-', 1)[1]))
    if not images:
        raise AnalysisError(f'{pdf}: rasterizer produced no page images')
    return images

def changed_mask(a, b, *, settings=None):
    if settings is None:
        settings = default_settings()
    red, green, blue = ImageChops.difference(a, b).split()
    return Image.eval(ImageChops.lighter(ImageChops.lighter(red, green), blue), lambda p: CHANNEL_MAX if p > settings['pixel_delta'] else 0)

def compare_pixels(a, b, ink_pixels=None, *, settings=None):
    """Changed pixels relative to the ink of these images, or of the full pages when `ink_pixels` is given."""
    if settings is None:
        settings = default_settings()
    a, b = (a.convert('RGB'), b.convert('RGB'))
    if a.size != b.size:
        return ({'size_match': False, 'changed_ink_fraction': 1.0}, None)
    delta = ImageChops.difference(a, b)
    ink = ImageChops.darker(a, b).convert('L').point(lambda p: CHANNEL_MAX if p < settings['ink_lightness_cutoff'] else 0)
    ink_count = sum(ink.histogram()[1:]) if ink_pixels is None else ink_pixels
    changed_count = sum(changed_mask(a, b, settings=settings).histogram()[1:])
    return ({'size_match': True, 'mean_absolute_channel_error': sum(ImageStat.Stat(delta).mean) / 3, 'changed_pixels': changed_count, 'ink_pixels': ink_count, 'changed_ink_fraction': changed_count / max(1, ink_count)}, delta)

def visible_text(full, text_free_page, *, settings=None):
    """Pixels that text changes on the page: what remains visible after clipping, occlusion and alpha."""
    if settings is None:
        settings = default_settings()
    full, text_free_page = (full.convert('RGB'), text_free_page.convert('RGB'))
    return changed_mask(full, text_free_page, settings=settings)

def word_boxes(lines, page, page_height, *, settings=None):
    """Pixel bounding boxes of the reference words on `page`."""
    if settings is None:
        settings = default_settings()
    scale = settings['resolution_dpi'] / settings['points_per_inch']
    boxes = []
    for line in lines:
        if line[0]['glyph'].page != page:
            continue
        words = []
        for glyph in line:
            if glyph['word_start'] or not words:
                words.append([])
            words[-1].append(glyph)
        for word in words:
            matrix = word[0]['glyph'].matrix
            em = math.hypot(*matrix[:2])
            ux, uy = (matrix[0] / em, matrix[1] / em)
            along = (word[0]['along'] - settings['word_box_em']['before'] * em, word[-1]['along'] + settings['word_box_em']['after'] * em)
            across = (word[0]['across'] - settings['word_box_em']['below'] * em, word[0]['across'] + settings['word_box_em']['above'] * em)
            xs = [(ux * s - uy * t) * scale for s in along for t in across]
            ys = [(page_height - (uy * s + ux * t)) * scale for s in along for t in across]
            boxes.append({'text': ''.join((glyph['glyph'].text for glyph in word)), 'box': (math.floor(min(xs)), math.floor(min(ys)), math.ceil(max(xs)), math.ceil(max(ys)))})
    return boxes

def compare_text_ink(reference, candidate, boxes, page, *, settings=None):
    if settings is None:
        settings = default_settings()
    mismatches = []
    for word in boxes:
        expected, actual = (sum(mask.crop(word['box']).histogram()[1:]) for mask in (reference, candidate))
        if abs(actual - expected) > max(settings['regional_ink_coverage_fraction'] * expected, settings['text_ink_floor_px']):
            mismatches.append({'page': page, 'text': word['text'], 'box_px': word['box'], 'reference_pixels': expected, 'candidate_pixels': actual})
    return mismatches

def close(a, b, tolerance):
    return a is None and b is None or a is not None and b is not None and len(a) == len(b) and all(abs(x - y) <= tolerance for x, y in zip(a, b))


def same_destination(a, b, tolerance):
    return a["page"] == b["page"] and abs(a["top"] - b["top"]) <= tolerance


def same_link(a, b, geometry, tolerance):
    if a["page"] != b["page"] or not close(a["rect"], b["rect"], geometry) or not close(a["quads"], b["quads"], geometry):
        return False
    if "uri" in a or "uri" in b:
        return a.get("uri") == b.get("uri")
    return same_destination(a["destination"], b["destination"], tolerance)


def outline_differences(reference, candidate, tolerance, path=()):
    """Bookmarks compared in order: title, open state of items with children, destination and children."""
    differences = []
    for index, (a, b) in enumerate(itertools.zip_longest(reference, candidate)):
        where = [*path, index]
        if a is None or b is None:
            differences.append({"bookmark": where, "reference": a and a["title"], "candidate": b and b["title"]})
            continue
        if a["title"] != b["title"] or (a["children"] and a["open"] != b["open"]) or not same_destination(a["destination"], b["destination"], tolerance):
            differences.append({"bookmark": where, "reference": {k: a[k] for k in ("title", "open", "destination")}, "candidate": {k: b[k] for k in ("title", "open", "destination")}})
        differences += outline_differences(a["children"], b["children"], tolerance, where)
    return differences


def compare_navigation(reference, candidate, *, settings):
    """Link annotations, outline, information dictionary, language and (when the reference has it) XMP.

    Links pair regardless of annotation order, which only sets the tab order; each needs the same page, target
    (URI, or destination page and top) and rectangle and quadrilaterals within tolerance."""
    geometry = settings["glyph_origin_pt"]
    tolerance = 0.5 + geometry
    differences = []
    unmatched = list(candidate["links"])
    for link in reference["links"]:
        match = next((other for other in unmatched if same_link(link, other, geometry, tolerance)), None)
        if match is None:
            differences.append({"kind": "missing link", "reference": link})
        else:
            unmatched.remove(match)
    differences += [{"kind": "extra link", "candidate": link} for link in unmatched]
    differences += [{"kind": "outline", **difference} for difference in outline_differences(reference["outline"], candidate["outline"], tolerance)]
    info = [{key: value for key, value in document["info"].items() if key not in {"Producer"}} for document in (reference, candidate)]
    if info[0] != info[1]:
        differences.append({"kind": "information dictionary", "reference": info[0], "candidate": info[1]})
    # BCP 47 language tags are case-insensitive.
    if (reference["lang"] or "").lower() != (candidate["lang"] or "").lower():
        differences.append({"kind": "language", "reference": reference["lang"], "candidate": candidate["lang"]})
    if reference["xmp"] is not None:
        expected = reference["xmp"]
        actual = {key: (candidate["xmp"] or {}).get(key) for key in expected}
        if expected != actual:
            differences.append({"kind": "XMP", "reference": expected, "candidate": actual})
    checked = bool(reference["links"] or candidate["links"] or reference["outline"] or candidate["outline"] or any(info) or reference["lang"] or candidate["lang"] or reference["xmp"] is not None)
    return {"navigation_checked": checked, "navigation_match": not differences, "navigation_differences": differences[:settings["mismatch_report_limit"]]}


def compare_pdfs(reference, candidate, prefix, *, output_dir, settings, reference_analysis=None, candidate_analysis=None, timeout_s=DEFAULT_COMMAND_TIMEOUT_S):
    if settings is None:
        settings = default_settings()
    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    reference_analysis = reference_analysis or analyse(reference, settings=settings, output_dir=output_dir, timeout_s=timeout_s)
    candidate_analysis = candidate_analysis or analyse(candidate, settings=settings, output_dir=output_dir, timeout_s=timeout_s)
    result = text_lines(reference_analysis['pages'], candidate_analysis['pages'])
    result.update(compare_rotated_lines(reference_analysis['rotated_lines'], candidate_analysis['rotated_lines'], settings=settings))
    result.update(glyph_check(reference_analysis['glyph_lines'], candidate_analysis['glyph_lines'], settings=settings))
    try:
        result.update(compare_navigation(pdf_content.navigation(reference), pdf_content.navigation(candidate), settings=settings))
    except pdf_content.PdfError as error:
        raise AnalysisError(f'{prefix}: PDF navigation analysis failed: {error}') from error
    result.update({'raster': [], 'text_free_raster': [], 'text_ink_mismatches': []})
    images = (
        raster(reference, settings=settings, output_dir=output_dir, timeout_s=timeout_s),
        raster(candidate, settings=settings, output_dir=output_dir, timeout_s=timeout_s),
        raster(text_free(reference, output_dir=output_dir), settings=settings, output_dir=output_dir, timeout_s=timeout_s),
        raster(text_free(candidate, output_dir=output_dir), settings=settings, output_dir=output_dir, timeout_s=timeout_s),
    )
    expected_counts = (len(reference_analysis['pages']), len(candidate_analysis['pages'])) * 2
    if any(len(group) != expected for group, expected in zip(images, expected_counts)):
        raise AnalysisError(f'{prefix}: raster page counts differ from inspected PDF pages')
    pages = zip(*images, reference_analysis['pages'])
    for index, (ap, bp, af, bf, page) in enumerate(pages):
        with Image.open(ap) as a, Image.open(bp) as b, Image.open(af) as fa, Image.open(bf) as fb:
            stats, delta = compare_pixels(a, b, settings=settings)
            result['raster'].append(stats)
            if delta:
                delta.save(output_dir / f'{prefix}.diff-{index + 1}.png')
            free_stats, free_delta = compare_pixels(fa, fb, stats.get('ink_pixels'), settings=settings)
            result['text_free_raster'].append(free_stats)
            if free_delta:
                free_delta.save(output_dir / f'{prefix}.text-free-diff-{index + 1}.png')
            if a.size == b.size:
                boxes = word_boxes(reference_analysis['glyph_lines'], index + 1, page['height'], settings=settings)
                result['text_ink_mismatches'] += compare_text_ink(visible_text(a, fa, settings=settings), visible_text(b, fb, settings=settings), boxes, index + 1, settings=settings)
    result['visual_match'] = result['page_match'] and all((p['changed_ink_fraction'] <= settings['changed_ink_fraction'] for p in result['text_free_raster']))
    result['text_ink_match'] = not result['text_ink_mismatches']
    result['text_ink_mismatches'] = result['text_ink_mismatches'][:settings['mismatch_report_limit']]
    result['match'] = result['navigation_match'] and result['visual_match'] and result['text_line_match'] and result['rotated_line_match'] and result['glyph_match'] and result['text_ink_match']
    return result


class CalibrationError(AnalysisError):
    pass


def calibrate(output_dir: Path, settings: dict) -> dict:
    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    timeout_s = settings['command_timeout_s']
    first_reference = output_dir / 'calibration.pdf'
    shutil.copyfile(HERE / 'assets/calibration.pdf', first_reference)
    calibration_glyphs = pdf_content.glyphs(first_reference)
    calibration_pages = extract(first_reference, glyph_lines(calibration_glyphs, settings=settings), settings=settings, output_dir=output_dir, timeout_s=timeout_s)
    synthetic_text_calibration = not calibration_glyphs
    if synthetic_text_calibration:
        raise CalibrationError('Independent calibration reference has no glyphs')
    calibration_lines = glyph_lines(calibration_glyphs, settings=settings)
    shifted = [[{**glyph, 'along': glyph['along'] + 2} for glyph in line] for line in calibration_lines]
    missing = [line[:-1] for line in calibration_lines]
    if not glyph_check(calibration_lines, calibration_lines, settings=settings)['glyph_match']:
        raise CalibrationError("Calibration failed: glyph_check(calibration_lines, calibration_lines, settings=settings)['glyph_match']")
    if not not glyph_check(calibration_lines, shifted, settings=settings)['glyph_match']:
        raise CalibrationError("Calibration failed: not glyph_check(calibration_lines, shifted, settings=settings)['glyph_match']")
    if not not glyph_check(calibration_lines, missing, settings=settings)['glyph_match']:
        raise CalibrationError("Calibration failed: not glyph_check(calibration_lines, missing, settings=settings)['glyph_match']")
    if not text_lines(calibration_pages, calibration_pages)['text_line_match']:
        raise CalibrationError("Calibration failed: text_lines(calibration_pages, calibration_pages)['text_line_match']")
    if not not text_lines(calibration_pages, calibration_pages[:-1])['page_match']:
        raise CalibrationError("Calibration failed: not text_lines(calibration_pages, calibration_pages[:-1])['page_match']")
    first = raster(first_reference, settings=settings, output_dir=output_dir, timeout_s=timeout_s)[0]
    with Image.open(first) as image:
        if not compare_pixels(image, image, settings=settings)[0]['changed_ink_fraction'] == 0:
            raise CalibrationError("Calibration failed: compare_pixels(image, image, settings=settings)[0]['changed_ink_fraction'] == 0")
    pixel_shift = Image.new('RGB', (200, 200), 'white')
    ImageDraw.Draw(pixel_shift).rectangle((20, 20, 39, 39), fill='black')
    if not compare_pixels(pixel_shift, ImageChops.offset(pixel_shift, 4, 0), settings=settings)[0]['changed_ink_fraction'] > settings['changed_ink_fraction']:
        raise CalibrationError("Calibration failed: compare_pixels(pixel_shift, ImageChops.offset(pixel_shift, 4, 0), settings=settings)[0]['changed_ink_fraction'] > settings['changed_ink_fraction']")
    if not compare_pixels(Image.new('RGB', (10, 10), 'black'), Image.new('RGB', (10, 10), 'navy'), settings=settings)[0]['changed_ink_fraction'] == 1.0:
        raise CalibrationError("Calibration failed: compare_pixels(Image.new('RGB', (10, 10), 'black'), Image.new('RGB', (10, 10), 'navy'), settings=settings)[0]['changed_ink_fraction'] == 1.0")
    return {'synthetic_glyph_for_textless_input': synthetic_text_calibration, 'identical_pass': True, 'two_point_glyph_shift_rejected': True, 'missing_glyph_rejected': True, 'missing_page_rejected': True, 'identical_pixels_pass': True, 'two_point_pixel_shift_rejected': True, 'colour_only_difference_rejected': True}
