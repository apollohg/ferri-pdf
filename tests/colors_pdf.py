"""Rendered CSS Color 4/5 colours and conic gradients against an independent oracle.

The oracle follows the CSS Color 4 §19 sample code and the §14.2.2 gamut-mapping pseudocode,
so the renderer's colour maths is checked against a separate implementation of the spec.
"""
import math
from pathlib import Path
import subprocess
import tempfile
import unittest

from PIL import Image
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / 'target/debug/ferri-pdf'
RASTER_DPI = 72
SWATCH_HEIGHT = 20
PAGE_MARGIN = 20
CONIC_SIZE = 200
CONIC_RADIUS = 60
# One 8-bit step of rounding plus one for the viewer's shading evaluation.
SOLID_TOLERANCE = 1
SHADING_TOLERANCE = 3

SRGB_TO_XYZ = [[506752 / 1228815, 87881 / 245763, 12673 / 70218],
               [87098 / 409605, 175762 / 245763, 12673 / 175545],
               [7918 / 409605, 87881 / 737289, 1001167 / 1053270]]
XYZ_TO_SRGB = [[12831 / 3959, -329 / 214, -1974 / 3959],
               [-851781 / 878810, 1648619 / 878810, 36519 / 878810],
               [705 / 12673, -2585 / 12673, 705 / 667]]
P3_TO_XYZ = [[608311 / 1250200, 189793 / 714400, 198249 / 1000160],
             [35783 / 156275, 247089 / 357200, 198249 / 2500400],
             [0, 32229 / 714400, 5220557 / 5000800]]
D50_TO_D65 = [[0.955473421488075, -0.02309845494876471, 0.06325924320057072],
              [-0.0283697093338637, 1.0099953980813041, 0.021041441191917323],
              [0.012314014864481998, -0.020507649298898964, 1.330365926242124]]
D50 = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585]
XYZ_TO_LMS = [[0.8190224379967030, 0.3619062600528904, -0.1288737815209879],
              [0.0329836539323885, 0.9292868615863434, 0.0361446663506424],
              [0.0481771893596242, 0.2642395317527308, 0.6335478284694309]]
LMS_TO_OKLAB = [[0.2104542683093140, 0.7936177747023054, -0.0040720430116193],
                [1.9779985324311684, -2.4285922420485799, 0.4505937096174110],
                [0.0259040424655478, 0.7827717124575296, -0.8086757549230774]]
LMS_TO_XYZ = [[1.2268798758459243, -0.5578149944602171, 0.2813910456659647],
              [-0.0405757452148008, 1.1122868032803170, -0.0717110580655164],
              [-0.0763729366746601, -0.4214933324022432, 1.5869240198367816]]
OKLAB_TO_LMS = [[1.0, 0.3963377773761749, 0.2158037573099136],
                [1.0, -0.1055613458156586, -0.0638541728258133],
                [1.0, -0.0894841775298119, -1.2914855480194092]]
JND = 0.02
GAMUT_EPSILON = 0.0001


def multiply(matrix, vector):
    return [sum(row[index] * vector[index] for index in range(3)) for row in matrix]


def linear_srgb(values):
    return [value / 12.92 if abs(value) <= 0.04045
            else math.copysign(((abs(value) + 0.055) / 1.055) ** 2.4, value) for value in values]


def gamma_srgb(values):
    return [math.copysign(1.055 * abs(value) ** (1 / 2.4) - 0.055, value) if abs(value) > 0.0031308
            else 12.92 * value for value in values]


def srgb_to_xyz(rgb):
    return multiply(SRGB_TO_XYZ, linear_srgb(rgb))


def xyz_to_srgb(xyz):
    return gamma_srgb(multiply(XYZ_TO_SRGB, xyz))


def xyz_to_oklab(xyz):
    lms = multiply(XYZ_TO_LMS, xyz)
    return multiply(LMS_TO_OKLAB, [math.copysign(abs(value) ** (1 / 3), value) for value in lms])


def oklab_to_xyz(oklab):
    return multiply(LMS_TO_XYZ, [value ** 3 for value in multiply(OKLAB_TO_LMS, oklab)])


def lab_to_xyz(lab):
    kappa, epsilon = 24389 / 27, 216 / 24389
    f1 = (lab[0] + 16) / 116
    f0, f2 = f1 + lab[1] / 500, f1 - lab[2] / 200
    xyz = [f0 ** 3 if f0 ** 3 > epsilon else (116 * f0 - 16) / kappa,
           ((lab[0] + 16) / 116) ** 3 if lab[0] > kappa * epsilon else lab[0] / kappa,
           f2 ** 3 if f2 ** 3 > epsilon else (116 * f2 - 16) / kappa]
    return multiply(D50_TO_D65, [value * white for value, white in zip(xyz, D50)])


def polar_to_rectangular(lightness, chroma, hue):
    return [lightness, chroma * math.cos(math.radians(hue)), chroma * math.sin(math.radians(hue))]


def oklab_to_oklch(oklab):
    return [oklab[0], math.hypot(oklab[1], oklab[2]), math.degrees(math.atan2(oklab[2], oklab[1])) % 360]


def in_gamut(rgb):
    return all(0 <= value <= 1 for value in rgb)


def clip(rgb):
    return [min(max(value, 0), 1) for value in rgb]


def gamut_map(xyz):
    """CSS Color 4 §14.2.2 binary search gamut mapping with local MINDE into sRGB."""
    srgb = xyz_to_srgb(xyz)
    if in_gamut(srgb):
        return srgb
    lightness, chroma, hue = oklab_to_oklch(xyz_to_oklab(xyz))
    if lightness >= 1:
        return clip(xyz_to_srgb(oklab_to_xyz([1, 0, 0])))
    if lightness <= 0:
        return [0, 0, 0]

    def to_srgb(color):
        return xyz_to_srgb(oklab_to_xyz(polar_to_rectangular(*color)))

    def error(clipped, current):
        return math.dist(xyz_to_oklab(srgb_to_xyz(clipped)), polar_to_rectangular(*current))

    current = [lightness, chroma, hue]
    clipped = clip(to_srgb(current))
    if error(clipped, current) < JND:
        return clipped
    low, high, low_in_gamut = 0, chroma, True
    while high - low > GAMUT_EPSILON:
        current[1] = (low + high) / 2
        if low_in_gamut and in_gamut(to_srgb(current)):
            low = current[1]
            continue
        clipped = clip(to_srgb(current))
        distance = error(clipped, current)
        if distance < JND:
            if JND - distance < GAMUT_EPSILON:
                return clipped
            low_in_gamut = False
            low = current[1]
        else:
            high = current[1]
    return clipped


def eight_bit(rgb):
    return tuple(round(min(max(value, 0), 1) * 255) for value in rgb)


def oklch_mix(first, second, progress):
    """Shorter-hue OkLCh interpolation of opaque sRGB colours (CSS Color 4 §13)."""
    a = oklab_to_oklch(xyz_to_oklab(srgb_to_xyz(first)))
    b = oklab_to_oklch(xyz_to_oklab(srgb_to_xyz(second)))
    if b[2] - a[2] > 180:
        a[2] += 360
    elif b[2] - a[2] < -180:
        b[2] += 360
    mixed = [a[index] + (b[index] - a[index]) * progress for index in range(3)]
    return oklab_to_xyz(polar_to_rectangular(*mixed))


def oklab_mix(first, second, progress):
    a = xyz_to_oklab(srgb_to_xyz(first))
    b = xyz_to_oklab(srgb_to_xyz(second))
    return oklab_to_xyz([a[index] + (b[index] - a[index]) * progress for index in range(3)])


RED, BLUE = [1, 0, 0], [0, 0, 1]
SWATCHES = [
    ('oklch(70% 0.1 200)', oklab_to_xyz(polar_to_rectangular(0.7, 0.1, 200))),
    ('oklch(70% 0.4 30)', oklab_to_xyz(polar_to_rectangular(0.7, 0.4, 30))),
    ('lab(50% 40 60)', lab_to_xyz([50, 40, 60])),
    ('lch(60% 90 200)', lab_to_xyz(polar_to_rectangular(60, 90, 200))),
    ('color(display-p3 1 0 0)', multiply(P3_TO_XYZ, linear_srgb([1, 0, 0]))),
    ('color(xyz-d65 0.2 0.3 0.4)', [0.2, 0.3, 0.4]),
    ('color-mix(in oklch, red, blue)', oklch_mix(RED, BLUE, 0.5)),
    ('color-mix(in oklab, red 25%, blue)', oklab_mix(RED, BLUE, 0.75)),
    ('oklch(from red l c calc(h + 180))',
     oklab_to_xyz(polar_to_rectangular(*(lambda c: [c[0], c[1], c[2] + 180])(
         oklab_to_oklch(xyz_to_oklab(srgb_to_xyz(RED))))))),
]


class ColorsPdf(unittest.TestCase):
    def render(self, root, css, body):
        source = root / 'input.html'
        output = root / 'output.pdf'
        source.write_text(f'''<!doctype html><style>
@page{{size:300pt 400pt;margin:{PAGE_MARGIN}pt}}
body{{margin:0}}div{{margin:0}}{css}</style>{body}''')
        subprocess.run([str(RENDERER), str(source), str(output)], check=True, capture_output=True)
        return output

    def raster(self, root, output):
        subprocess.run(['pdftoppm', '-r', str(RASTER_DPI), '-aaVector', 'no', '-singlefile', '-png',
                        str(output), str(root / 'ink')], check=True, capture_output=True)
        with Image.open(root / 'ink.png') as image:
            return image.convert('RGB')

    def assertNear(self, actual, expected, tolerance, label):
        self.assertTrue(all(abs(a - e) <= tolerance for a, e in zip(actual, expected)),
                        f'{label}: rendered {actual}, oracle {expected} (tolerance {tolerance})')

    def test_solid_colours_are_gamut_mapped_srgb(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-colors-') as directory:
            root = Path(directory)
            css = f'div{{height:{SWATCH_HEIGHT}pt;width:100pt}}' + ''.join(
                f'#s{index}{{background:{value}}}' for index, (value, _) in enumerate(SWATCHES))
            body = ''.join(f'<div id=s{index}></div>' for index in range(len(SWATCHES)))
            with self.raster(root, self.render(root, css, body)) as pixels:
                for index, (value, xyz) in enumerate(SWATCHES):
                    centre = (PAGE_MARGIN + 50, PAGE_MARGIN + SWATCH_HEIGHT * index + SWATCH_HEIGHT // 2)
                    self.assertNear(pixels.getpixel(centre), eight_bit(gamut_map(xyz)),
                                    SOLID_TOLERANCE, value)

    def test_conic_gradients_sweep_clockwise_from_their_start_angle(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-conic-') as directory:
            root = Path(directory)
            for start in (0, 90):
                with self.subTest(start=start):
                    css = (f'div{{width:{CONIC_SIZE}pt;height:{CONIC_SIZE}pt;'
                           f'background:conic-gradient(from {start}deg,red,blue)}}')
                    with self.raster(root, self.render(root, css, '<div></div>')) as pixels:
                        centre = PAGE_MARGIN + CONIC_SIZE / 2
                        for angle in (30, 100, 170, 250, 330):
                            # CSS angles run clockwise from the top of the page.
                            point = (round(centre + CONIC_RADIUS * math.sin(math.radians(angle))),
                                     round(centre - CONIC_RADIUS * math.cos(math.radians(angle))))
                            turn = ((angle - start) % 360) / 360
                            expected = eight_bit([RED[i] + (BLUE[i] - RED[i]) * turn for i in range(3)])
                            self.assertNear(pixels.getpixel(point), expected, SHADING_TOLERANCE,
                                            f'from {start}deg at {angle}deg')

    def test_oklab_gradients_follow_oklab_interpolation(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-oklab-') as directory:
            root = Path(directory)
            width = 200
            css = f'div{{width:{width}pt;height:20pt;background:linear-gradient(in oklab to right,red,blue)}}'
            with self.raster(root, self.render(root, css, '<div></div>')) as pixels:
                for step in range(1, 10):
                    x = PAGE_MARGIN + width * step / 10
                    expected = eight_bit(gamut_map(oklab_mix(RED, BLUE, (x + 0.5 - PAGE_MARGIN) / width)))
                    self.assertNear(pixels.getpixel((int(x), PAGE_MARGIN + 10)), expected,
                                    SHADING_TOLERANCE, f'{step / 10:.1f} along the line')

    def test_a_dark_root_scheme_darkens_the_canvas_and_initial_text(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-scheme-') as directory:
            root = Path(directory)
            output = self.render(root, ':root{color-scheme:dark}p{margin:0;font-size:20pt}', '<p>A</p>')
            with self.raster(root, output) as pixels:
                self.assertEqual(pixels.getpixel((150, 300)), (0x12, 0x12, 0x12),
                                 'the page area shows the dark Canvas surface')
                self.assertEqual(pixels.getpixel((5, 5)), (255, 255, 255),
                                 'the page margin outside the canvas stays paper white')
            fills = {glyph.fill[:3] for glyph in pdf_content.inspect(output)['glyphs']}
            self.assertEqual(fills, {(1.0, 1.0, 1.0)}, 'initial color is the dark CanvasText')


if __name__ == '__main__':
    unittest.main(verbosity=2)
