use crate::resources::ResourceResolver;
use crate::style::GeneratedContent;
use crate::style::{
    Display, Style, StyleError, Stylesheet, cascade::Origin, computed::cascade, declaration,
    grammar::Property,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MarginLocation {
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
    LeftTop,
    LeftMiddle,
    LeftBottom,
    RightTop,
    RightMiddle,
    RightBottom,
    TopLeftCorner,
    TopRightCorner,
    BottomLeftCorner,
    BottomRightCorner,
}
impl MarginLocation {
    pub(crate) const ALL: [Self; 16] = [
        Self::TopLeft,
        Self::TopCenter,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomCenter,
        Self::BottomRight,
        Self::LeftTop,
        Self::LeftMiddle,
        Self::LeftBottom,
        Self::RightTop,
        Self::RightMiddle,
        Self::RightBottom,
        Self::TopLeftCorner,
        Self::TopRightCorner,
        Self::BottomLeftCorner,
        Self::BottomRightCorner,
    ];
    pub(super) fn parse(name: &str) -> Option<Self> {
        let names = [
            "top-left",
            "top-center",
            "top-right",
            "bottom-left",
            "bottom-center",
            "bottom-right",
            "left-top",
            "left-middle",
            "left-bottom",
            "right-top",
            "right-middle",
            "right-bottom",
            "top-left-corner",
            "top-right-corner",
            "bottom-left-corner",
            "bottom-right-corner",
        ];
        names
            .iter()
            .position(|candidate| name.eq_ignore_ascii_case(candidate))
            .map(|i| Self::ALL[i])
    }
    /// The default painting order: from the top-left corner clockwise (CSS Page 3 §3.1).
    pub(crate) fn painting_order(self) -> usize {
        const ORDER: [MarginLocation; 16] = [
            MarginLocation::TopLeftCorner,
            MarginLocation::TopLeft,
            MarginLocation::TopCenter,
            MarginLocation::TopRight,
            MarginLocation::TopRightCorner,
            MarginLocation::RightTop,
            MarginLocation::RightMiddle,
            MarginLocation::RightBottom,
            MarginLocation::BottomRightCorner,
            MarginLocation::BottomRight,
            MarginLocation::BottomCenter,
            MarginLocation::BottomLeft,
            MarginLocation::BottomLeftCorner,
            MarginLocation::LeftBottom,
            MarginLocation::LeftMiddle,
            MarginLocation::LeftTop,
        ];
        ORDER
            .iter()
            .position(|location| *location == self)
            .expect("every margin location has a painting order")
    }
    fn align(self) -> &'static str {
        match self {
            Self::TopLeft | Self::BottomLeft | Self::TopRightCorner | Self::BottomRightCorner => {
                "left"
            }
            Self::TopRight | Self::BottomRight | Self::TopLeftCorner | Self::BottomLeftCorner => {
                "right"
            }
            _ => "center",
        }
    }
    fn vertical_align(self) -> &'static str {
        match self {
            Self::LeftTop | Self::RightTop => "top",
            Self::LeftBottom | Self::RightBottom => "bottom",
            _ => "middle",
        }
    }
}
#[derive(Clone)]
pub(crate) struct PageMargin {
    pub location: MarginLocation,
    pub content: GeneratedContent,
    pub style: Style,
}
impl Stylesheet {
    #[cfg(test)]
    pub(crate) fn page_margins(&self, root: &Style) -> Result<Vec<PageMargin>, StyleError> {
        self.page_margins_with_resolver(root, &crate::resources::FileResolver::default())
    }

    /// Margin boxes use the first page area as the viewport.
    #[cfg(test)]
    pub(crate) fn page_margins_with_resolver(
        &self,
        root: &Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<Vec<PageMargin>, StyleError> {
        let page = self.page_style_with_resolver(root, resolver)?;
        self.page_margins_from_page(&page, resolver)
    }

    #[cfg(test)]
    pub(crate) fn page_style_with_resolver(
        &self,
        root: &Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<Style, StyleError> {
        crate::style::resolution::page(
            self,
            root,
            self.match_page(&self.first_page),
            Some(self.page_area()),
            resolver,
        )
        .map(|(_, page)| page)
    }

    #[cfg(test)]
    pub(crate) fn page_margins_from_page(
        &self,
        page: &Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<Vec<PageMargin>, StyleError> {
        let rules = self.match_page(&self.first_page);
        self.page_margins_for(page, &rules, self.page_area(), resolver)
    }

    pub(super) fn page_margins_for(
        &self,
        page: &Style,
        rules: &crate::style::page_rules::MatchedPageRules,
        viewport: [f32; 2],
        resolver: &dyn ResourceResolver,
    ) -> Result<Vec<PageMargin>, StyleError> {
        let mut margins = Vec::new();
        for location in MarginLocation::ALL {
            let declared = &rules.margins[location as usize];
            if declared.is_empty() {
                continue;
            }
            let mut declarations = Vec::new();
            declaration::append(
                &mut declarations,
                Origin::UserAgent,
                Property::TextAlign,
                location.align(),
            )?;
            declaration::append(
                &mut declarations,
                Origin::UserAgent,
                Property::VerticalAlign,
                location.vertical_align(),
            )?;
            let mut winners = cascade(&declarations);
            winners.extend(declared.clone());
            let (style, content) = self.page_area_style(
                page,
                winners,
                crate::style::generated::ContentContext::Margin,
                viewport,
                resolver,
            )?;
            let Some(content) = content else {
                continue;
            };
            margins.push(PageMargin {
                location,
                content,
                style,
            });
        }
        Ok(margins)
    }

    /// The style of a box of the page context, a page-margin box or the footnote area, and
    /// its generated content. Display does not apply to either.
    pub(super) fn page_area_style(
        &self,
        page: &Style,
        winners: crate::style::cascade::Cascaded,
        context: crate::style::generated::ContentContext,
        viewport: [f32; 2],
        resolver: &dyn ResourceResolver,
    ) -> Result<(Style, Option<GeneratedContent>), StyleError> {
        let mut style = page.anonymous(Display::Block);
        style.content_context = context;
        let (mut style, mut winners) = crate::style::resolution::resolve_values(
            self,
            style,
            page,
            winners,
            resolver,
            &crate::style::resolution::ValueContext {
                tree: crate::style::units::Tree::Root,
                viewport,
                page_font: false,
            },
        )?;
        let inherited = self.inherited(page, viewport, resolver);
        let content = Self::generated(&mut winners, &mut style, page, &inherited)?;
        let relative = self.text_basis(&mut style, page, &mut winners, inherited, resolver)?;
        winners.remove(&Property::Display);
        Ok((
            Self::apply_properties(style, page, winners, &relative)?,
            content,
        ))
    }
}
