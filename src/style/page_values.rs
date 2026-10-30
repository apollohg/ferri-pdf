use crate::style::{
    MILLIMETRES_PER_INCH, POINTS_PER_INCH, StyleError,
    box_values::unsupported,
    component::Component,
    grammar::Keyword,
    units::{Axis, Relative},
};

const LETTER_PT: [f32; 2] = [612.0, 792.0];
const LEGAL_PT: [f32; 2] = [612.0, 1008.0];
const LEDGER_PT: [f32; 2] = [792.0, 1224.0];
const ISO_SIZES_MM: &[(Keyword, [f32; 2])] = &[
    (Keyword::A3, [297.0, 420.0]),
    (Keyword::A4, [210.0, 297.0]),
    (Keyword::A5, [148.0, 210.0]),
    (Keyword::B4, [250.0, 353.0]),
    (Keyword::B5, [176.0, 250.0]),
    (Keyword::JisB4, [257.0, 364.0]),
    (Keyword::JisB5, [182.0, 257.0]),
];
/// `[ <page-size> || [ portrait | landscape ] ]` slots.
const PAGE_SIZE_SLOT: usize = 0;
const ORIENTATION_SLOT: usize = 1;
const CROP_MARK_SLOT: usize = 0;
const CROSS_MARK_SLOT: usize = 1;
const AUTOMATIC_CROP_BLEED_PT: f32 = 6.0;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum Page {
    #[default]
    Auto,
    Name(std::sync::Arc<str>),
}
impl Page {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.unwrap() {
            Component::Keyword(Keyword::Auto) => Ok(Self::Auto),
            Component::Ident(name) => Ok(Self::Name(std::sync::Arc::from(name.as_ref()))),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct PageMarks {
    pub crop: bool,
    pub cross: bool,
}
impl PageMarks {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::None) {
            return Ok(Self::default());
        }
        let marks = Self {
            crop: value.slot(CROP_MARK_SLOT).is_some(),
            cross: value.slot(CROSS_MARK_SLOT).is_some(),
        };
        if marks.crop || marks.cross {
            Ok(marks)
        } else {
            Err(unsupported(value))
        }
    }
}

/// `page-orientation` (CSS Paged Media 3 §7.1.2): the quarter turn applied to the laid-out
/// page when the output medium displays it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageOrientation {
    #[default]
    Upright,
    RotateLeft,
    RotateRight,
}
impl PageOrientation {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Upright) => Ok(Self::Upright),
            Some(Keyword::RotateLeft) => Ok(Self::RotateLeft),
            Some(Keyword::RotateRight) => Ok(Self::RotateRight),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub(crate) struct PageProduction {
    pub bleed: f32,
    pub marks: PageMarks,
    pub orientation: PageOrientation,
}

impl crate::style::Stylesheet {
    pub(super) fn page_production_for(
        &self,
        page: &crate::style::Style,
        rules: &crate::style::page_rules::MatchedPageRules,
        viewport: [f32; 2],
        resolver: &dyn crate::resources::ResourceResolver,
    ) -> Result<PageProduction, StyleError> {
        use crate::style::grammar::Descriptor;
        let marks = rules
            .descriptor_value(self, Descriptor::PageMarks, page, viewport, resolver)?
            .0
            .as_deref()
            .map(PageMarks::from_component)
            .transpose()?
            .unwrap_or_default();
        let value = rules
            .descriptor_value(self, Descriptor::PageBleed, page, viewport, resolver)?
            .0;
        let bleed = match value.as_deref() {
            Some(value) if value.keyword() != Some(Keyword::Auto) => {
                crate::style::values::length(value, &self.inherited(page, viewport, resolver))?
            }
            _ if marks.crop => AUTOMATIC_CROP_BLEED_PT,
            _ => 0.0,
        };
        let orientation = rules
            .descriptor_value(
                self,
                Descriptor::PagePageOrientation,
                page,
                viewport,
                resolver,
            )?
            .0
            .as_deref()
            .map(PageOrientation::from_component)
            .transpose()?
            .unwrap_or_default();
        Ok(PageProduction {
            bleed,
            marks,
            orientation,
        })
    }
}

/// CSS Paged Media 3 `size`; `auto` uses configured paper. Font-relative lengths use the page
/// context's font.
pub(super) fn page_size(
    value: &Component,
    relative: &Relative<'_>,
    paper: [f32; 2],
) -> Result<[f32; 2], StyleError> {
    Ok([
        page_dimension(value, Axis::Width, relative, paper)?,
        page_dimension(value, Axis::Height, relative, paper)?,
    ])
}

pub(super) fn page_dimension(
    value: &Component,
    axis: Axis,
    relative: &Relative<'_>,
    paper: [f32; 2],
) -> Result<f32, StyleError> {
    let index = match axis {
        Axis::Width => 0,
        Axis::Height => 1,
        _ => unreachable!("page dimension is a physical axis"),
    };
    if value.keyword() == Some(Keyword::Auto) {
        return Ok(paper[index]);
    }
    if value.slots().is_empty() {
        let items = value.items();
        let item = match items.len() {
            1 => &items[0],
            2 => &items[index],
            _ => return Err(unsupported(value)),
        };
        let size = crate::style::values::length(item, relative)?;
        if size < 0.0 {
            return Err("page dimensions must be nonnegative".into());
        }
        return Ok(size);
    }
    let mut size = match value.slot(PAGE_SIZE_SLOT).and_then(Component::keyword) {
        None => paper,
        Some(Keyword::Letter) => LETTER_PT,
        Some(Keyword::Legal) => LEGAL_PT,
        Some(Keyword::Ledger) => LEDGER_PT,
        Some(name) => ISO_SIZES_MM
            .iter()
            .find(|(iso, _)| *iso == name)
            .map(|(_, mm)| mm.map(|v| v * POINTS_PER_INCH / MILLIMETRES_PER_INCH))
            .ok_or_else(|| unsupported(value))?,
    };
    if let Some(orientation) = value.slot(ORIENTATION_SLOT).and_then(Component::keyword) {
        let short = size[0].min(size[1]);
        let long = size[0].max(size[1]);
        size = if orientation == Keyword::Landscape {
            [long, short]
        } else {
            [short, long]
        };
    }
    Ok(size[index])
}
