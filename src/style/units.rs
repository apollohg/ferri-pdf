//! CSS Values 4 §6–§7 dimension units and what relative lengths resolve against.
use crate::fonts::{
    FALLBACK_IDEOGRAPH_ADVANCE_EM, FALLBACK_ZERO_ADVANCE_EM, FaceMetrics, FontAdjustments,
    FontBook, FontError, FontId, FontSelection, InitialFont, SizeMetric, WATER_IDEOGRAPH, ZERO,
    initial_font,
};
use crate::resources::ResourceResolver;
use crate::style::{
    DEFAULT_FONT_SIZE, DEFAULT_PAGE, DEFAULT_PAGE_AREA, MILLIMETRES_PER_CENTIMETRE,
    MILLIMETRES_PER_INCH, POINTS_PER_INCH, POINTS_PER_PIXEL, StyleError, text_values::LineHeight,
};
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
    /// The nearest query container's content box in the axis, or else the small viewport
    /// (CSS Conditional 5 §7).
    Container(Axis),
    ViewportLogical(bool),
    ContainerLogical(bool),
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
        adjustments: FontAdjustments,
        /// The computed font size, which selects optical sizes and adjusted sizes.
        size: f32,
    },
}

/// Em measures and the used size per computed size of the face that has them.
type Scaled<T> = (T, f32);

/// A font list measured on first use, so fonts no unit needs are never loaded.
#[derive(Clone)]
pub(crate) struct Faces<'a> {
    source: Source<'a>,
    first: OnceCell<Scaled<FaceMetrics>>,
    zero: OnceCell<Option<Scaled<f32>>>,
    water: OnceCell<Option<Scaled<f32>>>,
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
        adjustments: FontAdjustments,
        size: f32,
    ) -> Self {
        Self::new(Source::Book {
            book,
            resolver,
            candidates,
            selection,
            adjustments,
            size,
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
    fn first(&self) -> Result<Scaled<FaceMetrics>, FontError> {
        if let Some(metrics) = self.first.get() {
            return Ok(*metrics);
        }
        let metrics = match &self.source {
            Source::Initial(font) => (
                font.as_deref().unwrap_or_else(|| initial_font()).metrics(),
                1.0,
            ),
            Source::Book {
                book,
                resolver,
                candidates,
                selection,
                adjustments,
                size,
            } => {
                book.first_available_metrics(candidates, *selection, adjustments, *size, *resolver)?
            }
        };
        Ok(*self.first.get_or_init(|| metrics))
    }
    /// The advance of `ch`, in em, in the font that renders it.
    fn advance(
        &self,
        ch: char,
        cell: &OnceCell<Option<Scaled<f32>>>,
        vertical: bool,
    ) -> Result<Option<Scaled<f32>>, FontError> {
        if let Some(advance) = cell.get() {
            return Ok(*advance);
        }
        let advance = match &self.source {
            Source::Initial(font) => font
                .as_deref()
                .unwrap_or_else(|| initial_font())
                .advance((ch, vertical))
                .map(|advance| (advance, 1.0)),
            Source::Book {
                book,
                resolver,
                candidates,
                selection,
                adjustments,
                size,
            } => book.advance(
                candidates,
                *selection,
                adjustments,
                *size,
                (ch, vertical),
                *resolver,
            )?,
        };
        Ok(*cell.get_or_init(|| advance))
    }
}

pub(crate) trait FontUnits {
    fn per_unit(&self, unit: FontUnit) -> Result<f32, StyleError>;
    /// The `font-size-adjust` metric of the first available font, in em.
    fn size_metric(&self, metric: SizeMetric) -> Result<f32, StyleError>;
}

pub(crate) trait ViewportUnits {
    fn dimension(&self, axis: Axis) -> Result<f32, StyleError>;
    fn logical_dimension(&self, block: bool) -> Result<f32, StyleError>;
}

#[derive(Clone, Copy)]
pub(crate) enum Viewport<'a> {
    Known([f32; 2], bool),
    Deferred(&'a dyn ViewportUnits),
}
impl Viewport<'_> {
    fn logical_dimension(&self, block: bool) -> Result<f32, StyleError> {
        match self {
            Self::Deferred(source) => source.logical_dimension(block),
            Self::Known(_, vertical) => self.dimension(if block == *vertical {
                Axis::Width
            } else {
                Axis::Height
            }),
        }
    }
    fn dimension(&self, axis: Axis) -> Result<f32, StyleError> {
        match self {
            Self::Deferred(source) => source.dimension(axis),
            Self::Known([width, height], _) => Ok(match axis {
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
#[expect(
    clippy::large_enum_variant,
    reason = "font bases are stack values; boxing each line basis adds allocations during style resolution"
)]
pub(crate) enum Line<'a> {
    Known {
        height: LineHeight,
        size: f32,
        faces: Box<Faces<'a>>,
    },
    Deferred(&'a dyn FontUnits),
}
impl Line<'_> {
    pub(crate) fn initial(font: Option<Arc<InitialFont>>) -> Self {
        Self::Known {
            height: LineHeight::Normal,
            size: DEFAULT_FONT_SIZE,
            faces: Box::new(Faces::initial(font)),
        }
    }
    fn points(&self) -> Result<f32, StyleError> {
        match self {
            Self::Deferred(source) => source.per_unit(FontUnit::Lh),
            Self::Known {
                height,
                size,
                faces,
            } => {
                if let Some(height) = height.resolve(*size) {
                    return Ok(height);
                }
                let (metrics, scale) = faces.first()?;
                Ok(metrics.normal_line_height() * scale * size)
            }
        }
    }
}

/// Font metrics can be concrete or resolved on demand while constructing the first page.
#[derive(Clone)]
#[expect(
    clippy::large_enum_variant,
    reason = "bases are built on the stack for every element; boxing would allocate for each"
)]
pub(crate) enum FontBasis<'a> {
    Known {
        vertical: [bool; 2],
        size: f32,
        faces: Faces<'a>,
        line: Line<'a>,
    },
    Deferred(&'a dyn FontUnits),
}
impl<'a> FontBasis<'a> {
    pub(crate) fn with_initial_font(font: Option<Arc<InitialFont>>) -> Self {
        Self::Known {
            vertical: [false; 2],
            size: DEFAULT_FONT_SIZE,
            faces: Faces::initial(font.clone()),
            line: Line::initial(font),
        }
    }
    pub(crate) fn with_vertical_units(mut self, vertical: [bool; 2]) -> Self {
        if let Self::Known {
            vertical: units, ..
        } = &mut self
        {
            *units = vertical;
        }
        self
    }
    pub(crate) fn size(&self) -> Result<f32, StyleError> {
        match self {
            Self::Known { size, .. } => Ok(*size),
            Self::Deferred(source) => source.per_unit(FontUnit::Em),
        }
    }
    pub(crate) fn math_scaling(&self) -> Result<Option<[f32; 2]>, StyleError> {
        match self {
            Self::Known { faces, .. } => Ok(faces.first()?.0.math_scaling),
            Self::Deferred(_) => Err(StyleError::Unsupported(
                "font-size: math with deferred page font metrics".into(),
            )),
        }
    }
    pub(crate) fn into_line(self) -> Line<'a> {
        match self {
            Self::Known { line, .. } => line,
            Self::Deferred(source) => Line::Deferred(source),
        }
    }
    pub(crate) fn per_unit(&self, unit: FontUnit) -> Result<f32, StyleError> {
        let (size, faces, line, vertical) = match self {
            Self::Known {
                size,
                faces,
                line,
                vertical,
            } => (size, faces, line, vertical),
            Self::Deferred(source) => return source.per_unit(unit),
        };
        // `font-size-adjust` scales metric-based units but not `em` (CSS Fonts 5 §3.4).
        let scaled = |(em, scale): Scaled<f32>| em * scale;
        let em = match unit {
            FontUnit::Em => 1.0,
            FontUnit::Ex => {
                let (metrics, scale) = faces.first()?;
                metrics.x_height * scale
            }
            FontUnit::Cap => {
                let (metrics, scale) = faces.first()?;
                metrics.cap_height * scale
            }
            FontUnit::Ch => faces.advance(ZERO, &faces.zero, vertical[0])?.map_or(
                if vertical[0] {
                    FALLBACK_IDEOGRAPH_ADVANCE_EM
                } else {
                    FALLBACK_ZERO_ADVANCE_EM
                },
                scaled,
            ),
            FontUnit::Ic => faces
                .advance(WATER_IDEOGRAPH, &faces.water, vertical[1])?
                .map_or(FALLBACK_IDEOGRAPH_ADVANCE_EM, scaled),
            FontUnit::Lh => return line.points(),
        };
        Ok(em * size)
    }
    /// The `font-size-adjust` metric of the first available font, in em.
    pub(crate) fn size_metric(&self, metric: SizeMetric) -> Result<f32, StyleError> {
        match self {
            Self::Known { faces, .. } => Ok(faces.first()?.0.size_metric(metric)),
            Self::Deferred(source) => source.size_metric(metric),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Tree<'a> {
    None,
    Root,
    Element(crate::dom::Element<'a>),
}
impl Tree<'_> {
    pub(crate) fn siblings(self) -> Option<(usize, usize)> {
        const ROOT_POSITION: (usize, usize) = (1, 1);
        match self {
            Self::None => None,
            Self::Root => Some(ROOT_POSITION),
            Self::Element(element) => {
                Some(element.sibling_position(crate::dom::TreeRelation::Inheritance))
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
    pub(crate) work: Rc<crate::style::calc::Work>,
    /// CSS Page 3's compatibility choice for em/ex on the page's font-size only.
    pub(crate) initial_page_font_size: bool,
    pub(crate) initial_font: Option<Arc<InitialFont>>,
    /// The element's used colour scheme, for `light-dark()` and system colours.
    pub(crate) color_scheme: crate::style::ColorScheme,
    /// The document's counter styles, which `<counter-style>` values name; `None` before
    /// any document, which leaves the predefined ones.
    pub(crate) counter_styles: Option<Arc<crate::style::counter_style::CounterStyles>>,
    /// The element's computed `accent-color`, which `AccentColor` takes (CSS Color 4 §6.2);
    /// `None` is `auto`.
    pub(crate) accent_color: Option<crate::paint::AbsoluteColor>,
    /// The element's inclusive-ancestor query containers, and the sizes they measure.
    pub(crate) containers: crate::style::QueryContainers,
    pub(crate) query_sizes: Option<&'a crate::style::QuerySizes>,
    /// The element's measured anchors, which its anchor functions resolve against.
    pub(crate) anchoring: Option<crate::style::anchor_values::Anchoring<'a>>,
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
            viewport: Viewport::Known(DEFAULT_PAGE_AREA, false),
            paper: DEFAULT_PAGE,
            tree: Tree::None,
            grammar_only: false,
            work: Rc::default(),
            initial_page_font_size: false,
            initial_font,
            color_scheme: crate::style::ColorScheme::Light,
            counter_styles: None,
            accent_color: None,
            containers: crate::style::QueryContainers::default(),
            query_sizes: None,
            anchoring: None,
        }
    }
    pub(crate) fn media(paper: [f32; 2], font: Option<Arc<InitialFont>>) -> Self {
        Self {
            viewport: Viewport::Known(paper, false),
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
            Scale::ViewportLogical(block) => self.viewport.logical_dimension(block)? / PERCENT,
            Scale::ContainerLogical(block) => self.container_logical_dimension(block)? / PERCENT,
            Scale::Viewport(axis) => self.viewport.dimension(axis)? / PERCENT,
            Scale::Container(axis) => self.container_dimension(axis)? / PERCENT,
        })
    }
    /// CSS Conditional 5 §7: each axis without an eligible container uses the small viewport.
    fn container_dimension(&self, axis: Axis) -> Result<f32, StyleError> {
        if matches!(axis, Axis::Min | Axis::Max) {
            let inline = self.container_logical_dimension(false)?;
            let block = self.container_logical_dimension(true)?;
            return Ok(if axis == Axis::Min {
                inline.min(block)
            } else {
                inline.max(block)
            });
        }
        let [width, height] = [Axis::Width, Axis::Height].map(|axis| {
            self.query_sizes
                .and_then(|sizes| self.containers.unit_basis(sizes, axis))
        });
        let width = width.map_or_else(|| self.viewport.dimension(Axis::Width), Ok)?;
        let height = height.map_or_else(|| self.viewport.dimension(Axis::Height), Ok)?;
        Ok(match axis {
            Axis::Width => width,
            Axis::Height => height,
            Axis::Min => width.min(height),
            Axis::Max => width.max(height),
        })
    }
    fn container_logical_dimension(&self, block: bool) -> Result<f32, StyleError> {
        self.query_sizes
            .and_then(|sizes| self.containers.logical_unit_basis(sizes, block))
            .map_or_else(|| self.viewport.logical_dimension(block), Ok)
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
/// Paged media has no dynamic toolbars, so small, large and dynamic viewports coincide.
const fn viewport(name: &'static str, axis: Axis) -> (&'static str, Dimension) {
    length(name, Scale::Viewport(axis))
}
const fn container(name: &'static str, axis: Axis) -> (&'static str, Dimension) {
    length(name, Scale::Container(axis))
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
    length("vi", Scale::ViewportLogical(false)),
    length("svi", Scale::ViewportLogical(false)),
    length("lvi", Scale::ViewportLogical(false)),
    length("dvi", Scale::ViewportLogical(false)),
    length("vb", Scale::ViewportLogical(true)),
    length("svb", Scale::ViewportLogical(true)),
    length("lvb", Scale::ViewportLogical(true)),
    length("dvb", Scale::ViewportLogical(true)),
    viewport("vmin", Axis::Min),
    viewport("svmin", Axis::Min),
    viewport("lvmin", Axis::Min),
    viewport("dvmin", Axis::Min),
    viewport("vmax", Axis::Max),
    viewport("svmax", Axis::Max),
    viewport("lvmax", Axis::Max),
    viewport("dvmax", Axis::Max),
    container("cqw", Axis::Width),
    container("cqh", Axis::Height),
    length("cqi", Scale::ContainerLogical(false)),
    length("cqb", Scale::ContainerLogical(true)),
    container("cqmin", Axis::Min),
    container("cqmax", Axis::Max),
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
    if let Some(degrees) = crate::style::angles::degrees_per_unit(unit) {
        return Some(Dimension::Angle(degrees as f32));
    }
    DIMENSIONS
        .iter()
        .find(|(name, _)| unit.eq_ignore_ascii_case(name))
        .map(|(_, dimension)| *dimension)
}
