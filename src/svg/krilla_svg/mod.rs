/*!
An extension to krilla that allows rendering SVG files to a PDF file.

It is based on [usvg](https://github.com/linebender/resvg) and passes nearly the whole
resvg test suite. See the [examples]( https://github.com/LaurenzV/krilla/tree/main/crates/krilla-svg/examples)
directory for an example on how to use this crate in combination with krilla to convert SVG files
to PDF.
*/

#![allow(
    clippy::pedantic,
    clippy::too_many_arguments,
    elided_lifetimes_in_paths,
    reason = "inlined upstream code keeps its upstream style"
)]

use fontdb::Database;
use krilla::geom::{Rect, Size, Transform};
use krilla::paint::FillRule;
use krilla::surface::Surface;
use usvg::{Tree, fontdb};

use crate::svg::krilla_svg::text::Fonts;
use crate::svg::krilla_svg::util::RectExt;

mod clip_path;
mod filter;
mod group;
mod image;
mod mask;
mod path;
mod text;
mod util;

/// Settings that should be applied when converting a SVG.
#[derive(Copy, Clone, Debug)]
pub(crate) struct SvgSettings<'a> {
    /// Whether text should be embedded as properly selectable text. Otherwise,
    /// it will be drawn as outlined paths instead.
    pub embed_text: bool,
    /// How much filters, which will be converted to bitmaps, should be scaled. Higher values
    /// mean better quality, but also bigger file sizes.
    pub filter_scale: f32,
    /// Shared PNG data identities originating from decoded photos.
    pub jpeg_images: Option<&'a std::collections::HashSet<usize>>,
    /// Recoverable raster decoding failures while rendering this SVG.
    pub image_errors: Option<&'a std::cell::RefCell<std::collections::BTreeSet<String>>>,
}

impl Default for SvgSettings<'_> {
    fn default() -> Self {
        Self {
            embed_text: true,
            filter_scale: 4.0,
            jpeg_images: None,
            image_errors: None,
        }
    }
}

/// An extension trait for the `Surface` struct that allows you to draw SVGs onto a surface.
pub(crate) trait SurfaceExt {
    /// Draw a `usvg` tree onto a surface with the given size and settings.
    fn draw_svg(&mut self, tree: &Tree, size: Size, svg_settings: SvgSettings) -> Option<()>;
}

impl SurfaceExt for Surface<'_> {
    fn draw_svg(&mut self, tree: &Tree, size: Size, svg_settings: SvgSettings) -> Option<()> {
        let old_fill = self.get_fill().cloned();
        let old_stroke = self.get_stroke().cloned();

        let transform = Transform::from_scale(
            size.width() / tree.size().width(),
            size.height() / tree.size().height(),
        );
        self.push_transform(&transform);
        self.push_clip_path(
            &Rect::from_xywh(0.0, 0.0, tree.size().width(), tree.size().height())
                .unwrap()
                .to_clip_path(),
            &FillRule::NonZero,
        );
        render_tree(tree, svg_settings, self);
        self.pop();
        self.pop();

        self.set_fill(old_fill);
        self.set_stroke(old_stroke);

        Some(())
    }
}

struct ProcessContext<'a> {
    pub(crate) fonts: Fonts<'a>,
    svg_settings: SvgSettings<'a>,
}

impl<'a> ProcessContext<'a> {
    fn new(tree_fontdb: &'a Database, svg_settings: SvgSettings<'a>) -> Self {
        Self {
            fonts: Fonts::new(tree_fontdb),
            svg_settings,
        }
    }
}

pub(crate) fn render_tree(tree: &Tree, svg_settings: SvgSettings, surface: &mut Surface) {
    let mut fc = ProcessContext::new(tree.fontdb(), svg_settings);
    group::render(tree.root(), surface, &mut fc);
}
