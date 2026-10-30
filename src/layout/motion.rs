//! Viewport-dependent SVG offset references, shared by measurement and page effects.
use crate::document::svg::SvgDefinitions;
use crate::layout::{LayoutError, work::Budget};
use crate::paint::Contour;
use crate::resources::{ResourceBase, ResourceResolver};
use crate::style::{
    Style,
    motion::{Curve, OffsetPath, Reference},
};
use std::{cell::RefCell, collections::HashMap, sync::Arc};

type PathKey = (Box<str>, [u32; 2], crate::fonts::FontNamespace);
pub(super) struct Resources {
    definitions: SvgDefinitions,
    fonts: crate::images::SvgFonts,
    documents: RefCell<HashMap<Box<str>, Option<Arc<str>>>>,
    paths: RefCell<HashMap<PathKey, Option<Arc<Vec<Contour>>>>>,
}
impl Resources {
    pub(super) fn new(definitions: SvgDefinitions, fonts: &crate::fonts::FontBook) -> Self {
        Self {
            definitions,
            fonts: crate::images::SvgFonts::new(Arc::new(fonts.clone())),
            documents: Default::default(),
            paths: Default::default(),
        }
    }
    pub(super) fn resolve(
        &self,
        style: &Style,
        reference: &mut Reference,
        resolver: &dyn ResourceResolver,
        work: &Budget,
    ) -> Result<(), LayoutError> {
        work.spend(match &*style.motion.path {
            OffsetPath::None => 0,
            OffsetPath::Path {
                curve: Curve::Shape { shape, .. },
                ..
            } => match shape {
                crate::paint::BasicShape::Path { contours, .. } => {
                    contours.iter().map(|path| path.segments.len() + 1).sum()
                }
                crate::paint::BasicShape::Polygon { points, .. } => points.len(),
                _ => 1,
            },
            OffsetPath::Path { .. } => 1,
        })?;
        let OffsetPath::Path {
            curve: Curve::Reference(url),
            reference: coordinate,
        } = &*style.motion.path
        else {
            return Ok(());
        };
        let rect = reference.resolve(*coordinate);
        let size = [rect.width, rect.height].map(|value| value / crate::paint::POINTS_PER_PIXEL);
        let namespace = style.element_font_namespace();
        let key = (url.clone(), size.map(f32::to_bits), namespace);
        if let Some(path) = self.paths.borrow().get(&key) {
            reference.path.clone_from(path);
            if let Some(path) = path {
                work.spend(path.iter().map(|path| path.segments.len() + 1).sum())?;
            }
            return Ok(());
        }
        work.spend(url.len().max(1))?;
        let (source, id) = if let Some((document, id)) = url.rsplit_once('#') {
            if let Some(definition) = self.definitions.motion(url) {
                (Some(definition.xml.clone()), id)
            } else if document.is_empty() {
                (None, id)
            } else {
                if !self.documents.borrow().contains_key(document) {
                    let source = resolver
                        .resolve(document, &ResourceBase::default())
                        .ok()
                        .and_then(|resource| String::from_utf8(resource.bytes).ok())
                        .map(Arc::<str>::from);
                    self.documents.borrow_mut().insert(document.into(), source);
                }
                (self.documents.borrow().get(document).cloned().flatten(), id)
            }
        } else {
            (None, "")
        };
        let path = if let Some(source) = source {
            work.spend(source.len().max(1))?;
            let fonts = self.fonts.in_namespace(namespace);
            let provider = fonts.provider(Some(resolver));
            let mut options = crate::images::svg_options(crate::images::DEFAULT_OBJECT_SIZE)?;
            options.fontdb = fonts.database();
            let path = crate::svg::usvg::motion_path(&source, id, size, &options, Some(&provider));
            provider.finish().map_err(crate::style::StyleError::from)?;
            path.map(Arc::new)
        } else {
            None
        };
        if let Some(path) = &path {
            work.spend(path.iter().map(|path| path.segments.len() + 1).sum())?;
        }
        reference.path.clone_from(&path);
        self.paths.borrow_mut().insert(key, path);
        Ok(())
    }
}
