//! CSS Values 4 §6–§7 dimension units and what relative lengths resolve against.
use crate::{
    DEFAULT_FONT_SIZE, DEFAULT_PAGE, DEFAULT_PAGE_AREA, MILLIMETRES_PER_CENTIMETRE,
    MILLIMETRES_PER_INCH, POINTS_PER_INCH, POINTS_PER_PIXEL, StyleError, text_values::LineHeight,
};
use ferri_pdf_fonts::{
    FaceMetrics, FontBook, FontError, FontId, FontSelection, InitialFont, initial_font,
};
use ferri_pdf_resources::ResourceResolver;
use std::{cell::OnceCell, rc::Rc, sync::Arc};

const POINTS_PER_PICA: f32 = 12.0;
const QUARTERS_PER_MILLIMETRE: f32 = 4.0;
const POINTS_PER_MILLIMETRE: f32 = POINTS_PER_INCH / MILLIMETRES_PER_INCH;
const SECONDS_PER_MILLISECOND: f32 = 0.001;
const HERTZ_PER_KILOHERTZ: f32 = 1000.0;
const CSS_PIXELS_PER_INCH: f32 = 96.0;
const CENTIMETRES_PER_INCH: f32 = 2.54;
/// Viewport-percentage units are hundredths of the viewport.
const PERCENT: f32 = 100.0;
/// `ch` measures the advance of "0" and `ic` that of "水" (CSS Values 4 §6.1.1).
const ZERO: char = '0';
const WATER_IDEOGRAPH: char = '\u{6c34}';
/// CSS Values 4 §6.1.1 sizes where the font has no such glyph.
const FALLBACK_ZERO_ADVANCE_EM: f32 = 0.5;
const FALLBACK_IDEOGRAPH_ADVANCE_EM: f32 = 1.0;

/// A font-relative unit (CSS Values 4 §6.1.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FontUnit {
    Em,
    Ex,
    Cap,
    Ch,
    Ic,
    Lh,
}

/// The viewport dimension a viewport-percentage unit takes (CSS Values 4 §6.1.2).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Axis {
    Width,
    Height,
    Min,
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Scale {
    Points(f32),
    /// The element's own font, or the parent's in `font-size`.
    Font(FontUnit),
    /// The root element's font.
    Root(FontUnit),
    Viewport(Axis),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LengthUnit {
    pub(crate) name: &'static str,
    pub(crate) scale: Scale,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Length {
    pub(crate) value: f32,
    pub(crate) unit: LengthUnit,
}
impl Length {
    /// Every zero length, including unitless `0`.
    pub(crate) const ZERO: Self = Self {
        value: 0.0,
        unit: LengthUnit {
            name: "",
            scale: Scale::Points(1.0),
        },
    };
    pub(crate) fn points(self, relative: &Relative<'_>) -> Result<f32, StyleError> {
        let points = self.value * relative.per_unit(self.unit.scale)?;
        if !points.is_finite() {
            return Err("non-finite CSS length".into());
        }
        Ok(points)
    }
}

/// The faces a font measures with, as rendering selects them.
#[derive(Clone)]
enum Source<'a> {
    /// The initial family, which is bundled.
    Initial(Option<Arc<InitialFont>>),
    Book {
        book: &'a FontBook,
        resolver: &'a dyn ResourceResolver,
        candidates: Arc<[FontId]>,
        selection: FontSelection,
    },
}

/// A font list measured on first use, so fonts no unit needs are never loaded.
#[derive(Clone)]
pub(crate) struct Faces<'a> {
    source: Source<'a>,
    first: OnceCell<FaceMetrics>,
    zero: OnceCell<Option<f32>>,
    water: OnceCell<Option<f32>>,
}
impl<'a> Faces<'a> {
    pub(crate) fn initial(font: Option<Arc<InitialFont>>) -> Self {
        Self::new(Source::Initial(font))
    }
    pub(crate) fn book(
        book: &'a FontBook,
        resolver: &'a dyn ResourceResolver,
        candidates: Arc<[FontId]>,
        selection: FontSelection,
    ) -> Self {
        Self::new(Source::Book {
            book,
            resolver,
            candidates,
            selection,
        })
    }
    fn new(source: Source<'a>) -> Self {
        Self {
            source,
            first: OnceCell::new(),
            zero: OnceCell::new(),
            water: OnceCell::new(),
        }
    }
    /// The first available font's metrics, in em.
    fn first(&self) -> Result<FaceMetrics, FontError> {
        if let Some(metrics) = self.first.get() {
            return Ok(*metrics);
        }
        let metrics = match &self.source {
            Source::Initial(font) => font.as_deref().unwrap_or_else(|| initial_font()).metrics(),
            Source::Book {
                book,
                resolver,
                candidates,
                selection,
            } => book.first_available_metrics(candidates, *selection, *resolver)?,
        };
        Ok(*self.first.get_or_init(|| metrics))
    }
    /// The advance of `ch`, in em, in the font that renders it.
    fn advance(&self, ch: char, cell: &OnceCell<Option<f32>>) -> Result<Option<f32>, FontError> {
        if let Some(advance) = cell.get() {
            return Ok(*advance);
        }
        let advance = match &self.source {
            Source::Initial(font) => font
                .as_deref()
                .unwrap_or_else(|| initial_font())
                .advance(ch),
            Source::Book {
                book,
                resolver,
                candidates,
                selection,
            } => book.advance(candidates, *selection, ch, *resolver)?,
        };
        Ok(*cell.get_or_init(|| advance))
    }
}

pub(crate) trait FontUnits {
    fn per_unit(&self, unit: FontUnit) -> Result<f32, StyleError>;
}

pub(crate) trait ViewportUnits {
    fn dimension(&self, axis: Axis) -> Result<f32, StyleError>;
}

#[derive(Clone, Copy)]
pub(crate) enum Viewport<'a> {
    Known([f32; 2]),
    Deferred(&'a dyn ViewportUnits),
}
impl Viewport<'_> {
    fn dimension(&self, axis: Axis) -> Result<f32, StyleError> {
        match self {
            Self::Deferred(source) => source.dimension(axis),
            Self::Known([width, height]) => Ok(match axis {
                Axis::Width => *width,
                Axis::Height => *height,
                Axis::Min => width.min(*height),
                Axis::Max => width.max(*height),
            }),
        }
    }
}

/// The computed line height and the font that converts `normal` to an absolute length.
#[derive(Clone)]
pub(crate) enum Line<'a> {
    Known {
        height: LineHeight,
        size: f32,
        faces: Faces<'a>,
    },
    Deferred(&'a dyn FontUnits),
}
impl Line<'_> {
    pub(crate) fn initial(font: Option<Arc<InitialFont>>) -> Self {
        Self::Known {
            height: LineHeight::Normal,
            size: DEFAULT_FONT_SIZE,
            faces: Faces::initial(font),
        }
    }
    fn points(&self) -> Result<f32, StyleError> {
        match self {
            Self::Deferred(source) => source.per_unit(FontUnit::Lh),
            Self::Known {
                height,
                size,
                faces,
            } => match height.resolve(*size) {
                Some(height) => Ok(height),
                None => Ok(faces.first()?.normal_line_height() * size),
            },
        }
    }
}

/// Font metrics can be concrete or resolved on demand while constructing the first page.
#[derive(Clone)]
pub(crate) enum FontBasis<'a> {
    Known {
        size: f32,
        faces: Faces<'a>,
        line: Line<'a>,
    },
    Deferred(&'a dyn FontUnits),
}
impl<'a> FontBasis<'a> {
    pub(crate) fn with_initial_font(font: Option<Arc<InitialFont>>) -> Self {
        Self::Known {
            size: DEFAULT_FONT_SIZE,
            faces: Faces::initial(font.clone()),
            line: Line::initial(font),
        }
    }
    pub(crate) fn size(&self) -> Result<f32, StyleError> {
        match self {
            Self::Known { size, .. } => Ok(*size),
            Self::Deferred(source) => source.per_unit(FontUnit::Em),
        }
    }
    pub(crate) fn into_line(self) -> Line<'a> {
        match self {
            Self::Known { line, .. } => line,
            Self::Deferred(source) => Line::Deferred(source),
        }
    }
    pub(crate) fn per_unit(&self, unit: FontUnit) -> Result<f32, StyleError> {
        let (size, faces, line) = match self {
            Self::Known { size, faces, line } => (size, faces, line),
            Self::Deferred(source) => return source.per_unit(unit),
        };
        let em = match unit {
            FontUnit::Em => 1.0,
            FontUnit::Ex => faces.first()?.x_height,
            FontUnit::Cap => faces.first()?.cap_height,
            FontUnit::Ch => faces
                .advance(ZERO, &faces.zero)?
                .unwrap_or(FALLBACK_ZERO_ADVANCE_EM),
            FontUnit::Ic => faces
                .advance(WATER_IDEOGRAPH, &faces.water)?
                .unwrap_or(FALLBACK_IDEOGRAPH_ADVANCE_EM),
            FontUnit::Lh => return line.points(),
        };
        Ok(em * size)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Tree<'a> {
    None,
    Root,
    Element(ferri_pdf_dom::Element<'a>),
}
impl Tree<'_> {
    pub(crate) fn siblings(self) -> Option<(usize, usize)> {
        const ROOT_POSITION: (usize, usize) = (1, 1);
        match self {
            Self::None => None,
            Self::Root => Some(ROOT_POSITION),
            Self::Element(element) => {
                Some(element.sibling_position(ferri_pdf_dom::TreeRelation::Inheritance))
            }
        }
    }
}

/// What relative lengths resolve against: an element's font, the root's, and the viewport,
/// which in paged media is the page area (CSS Values 4 §6.1.2).
#[derive(Clone)]
pub(crate) struct Relative<'a> {
    pub(crate) font: FontBasis<'a>,
    pub(crate) root: FontBasis<'a>,
    pub(crate) viewport: Viewport<'a>,
    pub(crate) paper: [f32; 2],
    pub(crate) tree: Tree<'a>,
    pub(crate) grammar_only: bool,
    pub(crate) work: Rc<crate::calc::Work>,
    /// CSS Page 3's compatibility choice for em/ex on the page's font-size only.
    pub(crate) initial_page_font_size: bool,
    pub(crate) initial_font: Option<Arc<InitialFont>>,
}
impl Relative<'_> {
    /// Initial values and the default page area, for values computed without an element
    /// (Media Queries 4 §1.3).
    pub(crate) fn initial() -> Self {
        Self::with_initial_font(None)
    }
    pub(crate) fn with_initial_font(initial_font: Option<Arc<InitialFont>>) -> Self {
        Self {
            font: FontBasis::with_initial_font(initial_font.clone()),
            root: FontBasis::with_initial_font(initial_font.clone()),
            viewport: Viewport::Known(DEFAULT_PAGE_AREA),
            paper: DEFAULT_PAGE,
            tree: Tree::None,
            grammar_only: false,
            work: Rc::default(),
            initial_page_font_size: false,
            initial_font,
        }
    }
    pub(crate) fn media(paper: [f32; 2], font: Option<Arc<InitialFont>>) -> Self {
        Self {
            viewport: Viewport::Known(paper),
            paper,
            ..Self::with_initial_font(font)
        }
    }
    pub(crate) fn validation() -> Self {
        Self {
            tree: Tree::Root,
            ..Self::initial()
        }
    }
    /// Points per unit of `scale`.
    pub(crate) fn per_unit(&self, scale: Scale) -> Result<f32, StyleError> {
        Ok(match scale {
            Scale::Points(points) => points,
            Scale::Font(unit)
                if self.initial_page_font_size && matches!(unit, FontUnit::Em | FontUnit::Ex) =>
            {
                FontBasis::with_initial_font(self.initial_font.clone()).per_unit(unit)?
            }
            Scale::Font(unit) => self.font.per_unit(unit)?,
            Scale::Root(unit) => self.root.per_unit(unit)?,
            Scale::Viewport(axis) => self.viewport.dimension(axis)? / PERCENT,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Dimension {
    Length(LengthUnit),
    /// Degrees per unit.
    Angle(f32),
    /// Seconds per unit.
    Time(f32),
    /// Hertz per unit.
    Frequency(f32),
    /// Dots per CSS pixel per unit.
    Resolution(f32),
    Flex,
}

const fn length(name: &'static str, scale: Scale) -> (&'static str, Dimension) {
    (name, Dimension::Length(LengthUnit { name, scale }))
}
const fn font(name: &'static str, unit: FontUnit) -> (&'static str, Dimension) {
    length(name, Scale::Font(unit))
}
const fn root(name: &'static str, unit: FontUnit) -> (&'static str, Dimension) {
    length(name, Scale::Root(unit))
}
/// Paged media has no dynamic toolbars, so small, large and dynamic viewports coincide; the
/// only writing mode is horizontal, so the inline axis is the width; and without query
/// containers, container units use the small viewport (CSS Containment 3 §6).
const fn viewport(name: &'static str, axis: Axis) -> (&'static str, Dimension) {
    length(name, Scale::Viewport(axis))
}

const DIMENSIONS: &[(&str, Dimension)] = &[
    length("px", Scale::Points(POINTS_PER_PIXEL)),
    length("pt", Scale::Points(1.0)),
    length("pc", Scale::Points(POINTS_PER_PICA)),
    length("in", Scale::Points(POINTS_PER_INCH)),
    length(
        "cm",
        Scale::Points(POINTS_PER_MILLIMETRE * MILLIMETRES_PER_CENTIMETRE),
    ),
    length("mm", Scale::Points(POINTS_PER_MILLIMETRE)),
    length(
        "q",
        Scale::Points(POINTS_PER_MILLIMETRE / QUARTERS_PER_MILLIMETRE),
    ),
    font("em", FontUnit::Em),
    root("rem", FontUnit::Em),
    font("ex", FontUnit::Ex),
    root("rex", FontUnit::Ex),
    font("cap", FontUnit::Cap),
    root("rcap", FontUnit::Cap),
    font("ch", FontUnit::Ch),
    root("rch", FontUnit::Ch),
    font("ic", FontUnit::Ic),
    root("ric", FontUnit::Ic),
    font("lh", FontUnit::Lh),
    root("rlh", FontUnit::Lh),
    viewport("vw", Axis::Width),
    viewport("svw", Axis::Width),
    viewport("lvw", Axis::Width),
    viewport("dvw", Axis::Width),
    viewport("vh", Axis::Height),
    viewport("svh", Axis::Height),
    viewport("lvh", Axis::Height),
    viewport("dvh", Axis::Height),
    viewport("vi", Axis::Width),
    viewport("svi", Axis::Width),
    viewport("lvi", Axis::Width),
    viewport("dvi", Axis::Width),
    viewport("vb", Axis::Height),
    viewport("svb", Axis::Height),
    viewport("lvb", Axis::Height),
    viewport("dvb", Axis::Height),
    viewport("vmin", Axis::Min),
    viewport("svmin", Axis::Min),
    viewport("lvmin", Axis::Min),
    viewport("dvmin", Axis::Min),
    viewport("vmax", Axis::Max),
    viewport("svmax", Axis::Max),
    viewport("lvmax", Axis::Max),
    viewport("dvmax", Axis::Max),
    viewport("cqw", Axis::Width),
    viewport("cqh", Axis::Height),
    viewport("cqi", Axis::Width),
    viewport("cqb", Axis::Height),
    viewport("cqmin", Axis::Min),
    viewport("cqmax", Axis::Max),
    ("s", Dimension::Time(1.0)),
    ("ms", Dimension::Time(SECONDS_PER_MILLISECOND)),
    ("hz", Dimension::Frequency(1.0)),
    ("khz", Dimension::Frequency(HERTZ_PER_KILOHERTZ)),
    ("dppx", Dimension::Resolution(1.0)),
    ("x", Dimension::Resolution(1.0)),
    ("dpi", Dimension::Resolution(1.0 / CSS_PIXELS_PER_INCH)),
    (
        "dpcm",
        Dimension::Resolution(CENTIMETRES_PER_INCH / CSS_PIXELS_PER_INCH),
    ),
    ("fr", Dimension::Flex),
];

/// Units are ASCII case-insensitive.
pub(crate) fn dimension(unit: &str) -> Option<Dimension> {
    if let Some(degrees) = crate::angles::degrees_per_unit(unit) {
        return Some(Dimension::Angle(degrees as f32));
    }
    DIMENSIONS
        .iter()
        .find(|(name, _)| unit.eq_ignore_ascii_case(name))
        .map(|(_, dimension)| *dimension)
}
