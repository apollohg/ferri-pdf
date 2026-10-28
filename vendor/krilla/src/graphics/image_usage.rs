use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pdf_writer::Chunk;

use crate::error::{KrillaError, KrillaResult};
use crate::geom::Transform;
use crate::graphics::image::{Image, ImageDpi};
use crate::util::{Deferred, SipHashable};

const POINTS_PER_INCH: f64 = 72.0;
const MAX_IMAGE_USAGE_WORK: usize = 1_000_000;

#[derive(Clone, Default)]
pub(crate) struct ImageUses(Option<Arc<[ImageUse]>>);

impl std::fmt::Debug for ImageUses {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ImageUses")
            .field(&self.0.as_ref().map_or(0, |uses| uses.len()))
            .finish()
    }
}

pub(crate) struct ImageUse {
    transform: Linear,
    content: Content,
}
enum Content {
    Image(Image),
    Stream(ImageUses),
}

impl ImageUse {
    pub fn image(image: Image, transform: Transform) -> Self {
        Self {
            transform: Linear::from_transform(transform),
            content: Content::Image(image),
        }
    }
    pub fn stream(uses: ImageUses, transform: Transform) -> Self {
        Self {
            transform: Linear::from_transform(transform),
            content: Content::Stream(uses),
        }
    }
}

impl ImageUses {
    pub fn new(uses: Vec<ImageUse>) -> Self {
        Self((!uses.is_empty()).then(|| uses.into()))
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }
    pub fn target_sizes(
        pages: &[Self],
        glyphs: &[Self],
        dpi: ImageDpi,
    ) -> KrillaResult<HashMap<u128, (u32, u32)>> {
        let mut pending: Vec<_> = pages
            .iter()
            .cloned()
            .map(|uses| (uses, Linear::IDENTITY, false))
            .chain(
                glyphs
                    .iter()
                    .cloned()
                    .map(|uses| (uses, Linear::IDENTITY, true)),
            )
            .collect();
        let mut visited = HashSet::new();
        let mut sizes: HashMap<u128, (u32, u32)> = HashMap::new();
        let mut work = 0;
        while let Some((uses, parent, retain_source)) = pending.pop() {
            let Some(uses) = uses.0 else {
                continue;
            };
            let key = (
                Arc::as_ptr(&uses) as *const ImageUse as usize,
                parent.0.map(f64::to_bits),
                retain_source,
            );
            if !visited.insert(key) {
                continue;
            }
            for usage in uses.iter() {
                if work == MAX_IMAGE_USAGE_WORK {
                    return Err(KrillaError::ImageProcessingLimit(MAX_IMAGE_USAGE_WORK));
                }
                work += 1;
                let transform = if retain_source {
                    Linear::IDENTITY
                } else {
                    parent.concat(usage.transform)
                };
                match &usage.content {
                    Content::Stream(child) => {
                        pending.push((child.clone(), transform, retain_source))
                    }
                    Content::Image(image) => {
                        let [a, b, c, d] = transform.0;
                        if !transform.0.iter().all(|value| value.is_finite()) {
                            return Err(KrillaError::Image(
                                image.clone(),
                                None,
                                "non-finite physical image transform".into(),
                            ));
                        }
                        let source = image.size();
                        if source.0 == 0 || source.1 == 0 {
                            return Err(KrillaError::Image(
                                image.clone(),
                                None,
                                "zero image dimension".into(),
                            ));
                        }
                        let pixels = |length: f64, source: u32| {
                            let samples = length * f64::from(dpi.get()) / POINTS_PER_INCH;
                            // Drawing matrices are f32; discard their final-ulp roundoff.
                            (samples - samples.abs() * f64::from(f32::EPSILON))
                                .ceil()
                                .clamp(1.0, f64::from(source)) as u32
                        };
                        let target = if retain_source {
                            source
                        } else {
                            (pixels(a.hypot(b), source.0), pixels(c.hypot(d), source.1))
                        };
                        sizes
                            .entry(image.sip_hash())
                            .and_modify(|size| {
                                size.0 = size.0.max(target.0);
                                size.1 = size.1.max(target.1);
                            })
                            .or_insert(target);
                    }
                }
            }
        }
        Ok(sizes)
    }
}

#[derive(Clone, Copy)]
struct Linear([f64; 4]);
impl Linear {
    const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0]);
    fn from_transform(value: Transform) -> Self {
        Self([value.sx(), value.ky(), value.kx(), value.sy()].map(f64::from))
    }
    fn concat(self, child: Self) -> Self {
        let [a, b, c, d] = self.0;
        let [e, f, g, h] = child.0;
        Self([a * e + c * f, b * e + d * f, a * g + c * h, b * g + d * h])
    }
}

pub(crate) struct DeferredImage {
    pub hash: u128,
    pub source_size: (u32, u32),
    pub encode: Box<dyn FnOnce((u32, u32)) -> Deferred<KrillaResult<Chunk>> + Send>,
}
