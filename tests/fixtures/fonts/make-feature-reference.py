"""Record independent HarfBuzz shaping of OpenType features that CSS font properties select."""
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path(__file__).resolve().parent
SOURCE_SERIF = FIXTURES / "SourceSerif4-Regular.ttf"
LIBERATION_SERIF = ROOT / "assets/fonts/LiberationSerif-Regular.ttf"
DEJAVU_SANS = ROOT / "assets/fonts/DejaVuSans.ttf"
SHAPER_OPTIONS = ["--output-format=json", "--no-glyph-names"]
# (name, font, text, comma-separated features, language)
SAMPLES = [
    ("default_figures", SOURCE_SERIF, "1011", "", "und"),
    ("tabular_figures", SOURCE_SERIF, "1011", "tnum", "und"),
    ("proportional_figures", SOURCE_SERIF, "1011", "pnum", "und"),
    ("oldstyle_figures", SOURCE_SERIF, "1011", "onum", "und"),
    ("oldstyle_proportional_figures", SOURCE_SERIF, "1011", "onum,pnum", "und"),
    ("slashed_zero", SOURCE_SERIF, "0", "zero", "und"),
    ("diagonal_fractions", SOURCE_SERIF, "1/2", "frac", "und"),
    ("ordinals", SOURCE_SERIF, "2a", "ordn", "und"),
    ("superscripts", SOURCE_SERIF, "2a", "sups", "und"),
    ("subscripts", SOURCE_SERIF, "20", "subs", "und"),
    ("all_small_caps", SOURCE_SERIF, "Ag", "c2sc,smcp", "und"),
    ("petite_caps_fallback", SOURCE_SERIF, "Ag", "smcp", "und"),
    ("small_caps_english", SOURCE_SERIF, "i", "smcp", "en"),
    ("small_caps_turkish", SOURCE_SERIF, "i", "smcp", "tr"),
    ("styleset_one", SOURCE_SERIF, "в", "ss01", "und"),
    ("styleset_two", SOURCE_SERIF, "б", "ss02", "und"),
    ("default_cyrillic", SOURCE_SERIF, "вб", "", "und"),
    ("both_stylesets", SOURCE_SERIF, "вб", "ss01,ss02", "und"),
    ("kerned", LIBERATION_SERIF, "AVA", "", "und"),
    ("unkerned", LIBERATION_SERIF, "AVA", "-kern", "und"),
    ("native_superscript", LIBERATION_SERIF, "45", "sups", "und"),
    ("plain_superscript_candidates", LIBERATION_SERIF, "4x", "", "und"),
    ("common_ligature", DEJAVU_SANS, "fi", "", "und"),
    ("no_common_ligature", DEJAVU_SANS, "fi", "-liga", "und"),
    ("discretionary_ligature", DEJAVU_SANS, "st", "dlig", "und"),
    ("historical_ligature", DEJAVU_SANS, "ſt", "hlig", "und"),
    ("stylistic_alternate", DEJAVU_SANS, "a", "salt=1", "und"),
    ("default_alternate", DEJAVU_SANS, "a", "", "und"),
]


def shape(font, text, features, language):
    result = subprocess.run(
        ["hb-shape", str(font), text, f"--features={features}", f"--language={language}",
         *SHAPER_OPTIONS],
        check=True, capture_output=True, text=True)
    return json.loads(result.stdout)


def units_per_em(font):
    result = subprocess.run(["hb-info", str(font), "--show-upem"], check=True,
                            capture_output=True, text=True)
    return int(result.stdout.split()[-1])


fonts = {path: {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "units_per_em": units_per_em(path)}
         for path in {font for _, font, *_ in SAMPLES}}
record = {
    "generator": Path(__file__).name,
    "shaper": subprocess.run(["hb-shape", "--version"], check=True, capture_output=True,
                             text=True).stdout.splitlines()[0],
    "shaper_options": SHAPER_OPTIONS,
    "fonts": {str(path.relative_to(ROOT)): data for path, data in sorted(fonts.items())},
    "samples": {
        name: {
            "font": str(font.relative_to(ROOT)),
            "text": text,
            "features": features,
            "language": language,
            "glyphs": [{"g": glyph["g"], "cl": glyph["cl"], "ax": glyph["ax"], "dy": glyph["dy"]}
                       for glyph in shape(font, text, features, language)],
        }
        for name, font, text, features, language in SAMPLES
    },
}
(FIXTURES / "FontFeatures-reference.json").write_text(json.dumps(record, indent=2) + "\n")
