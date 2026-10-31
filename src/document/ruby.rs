//! Ruby box generation and annotation pairing (CSS Ruby 1 §2). A ruby container's bases are
//! inline boxes of its inline formatting context, while each annotation is collected apart and
//! laid out as an atomic box of its own annotation-level formatting context (§2.1.1).
use crate::document::{
    BlockBox, BlockContent, BoxNavigation, DocumentError, inline::InlineCollector,
};
use crate::graphics::BoxDecoration;
use crate::style::{Display, RubyMerge, RubyPosition, Style, TextAlign, Visibility};
use std::ops::Range;
use std::sync::Arc;

/// A ruby container of a paragraph, as segments of bases paired with annotation levels.
#[derive(Clone, Debug)]
pub(crate) struct Ruby {
    /// The container's inline box.
    pub container: usize,
    pub segments: Vec<Segment>,
}
impl Ruby {
    /// The ruby with its inline box indices and annotation box indices offset.
    pub(super) fn shifted(&self, boxes: usize, annotations: usize) -> Self {
        Self {
            container: self.container + boxes,
            segments: self
                .segments
                .iter()
                .map(|segment| Segment {
                    bases: segment.bases.iter().map(|base| base + boxes).collect(),
                    levels: segment
                        .levels
                        .iter()
                        .map(|level| Level {
                            annotations: level
                                .annotations
                                .iter()
                                .map(|annotation| Annotation {
                                    block: annotation.block.map(|block| block + annotations),
                                    ..annotation.clone()
                                })
                                .collect(),
                            ..level.clone()
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

/// A ruby base container and the annotation containers paired with it (§2.3.1).
#[derive(Clone, Debug)]
pub(crate) struct Segment {
    /// The inline boxes of its ruby bases, one per column.
    pub bases: Vec<usize>,
    /// Its annotation levels, the first level first.
    pub levels: Vec<Level>,
}

/// An annotation container.
#[derive(Clone, Debug)]
pub(crate) struct Level {
    /// It sets the level's merging, distribution and overhang (§§4.2–5.1).
    pub style: Arc<Style>,
    /// It lies under the bases rather than over them (§4.1).
    pub under: bool,
    pub inter_character: bool,
    pub annotations: Vec<Annotation>,
}

/// A ruby annotation paired with the bases it spans (§2.3.2).
#[derive(Clone, Debug)]
pub(crate) struct Annotation {
    /// Its box among the paragraph's annotations; `None` where it has no contents.
    pub block: Option<usize>,
    /// The style of the annotation box, whose margin box its level contains (§3.1.2).
    pub style: Arc<Style>,
    /// The bases it spans. Inter-annotation white space spans the empty range at the gap
    /// before base `k` that it pairs with.
    pub bases: Range<usize>,
    /// Inter-annotation white space, a collapsible space in its level's font (§2.5).
    pub whitespace: bool,
    /// Hidden by `visibility: collapse`, or because its text is its base's (§2.4).
    pub hidden: bool,
}

/// The kind of a box a ruby structure part is, which classifies the intra-ruby white space
/// beside it (§2.2).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    BaseContainer,
    Base,
    Level,
    Annotation,
}

/// An open annotation, whose contents the collector holds while its base level waits.
struct OpenAnnotation {
    explicit: bool,
    outer: InlineCollector,
    style: Arc<Style>,
    text: String,
}

/// A part of the ruby structure open in its container, explicit (an element) or anonymous.
enum Part {
    /// A base container, whose style anonymous bases inherit.
    BaseContainer {
        explicit: bool,
        style: Arc<Style>,
    },
    Base {
        explicit: bool,
        inline_box: usize,
        text: String,
    },
    /// An annotation container, whose style anonymous annotations inherit.
    Level {
        explicit: bool,
        style: Arc<Style>,
    },
    Annotation(Box<OpenAnnotation>),
}
impl Part {
    fn explicit(&self) -> bool {
        match self {
            Self::BaseContainer { explicit, .. }
            | Self::Base { explicit, .. }
            | Self::Level { explicit, .. } => *explicit,
            Self::Annotation(annotation) => annotation.explicit,
        }
    }
    fn kind(&self) -> Kind {
        match self {
            Self::BaseContainer { .. } => Kind::BaseContainer,
            Self::Base { .. } => Kind::Base,
            Self::Level { .. } => Kind::Level,
            Self::Annotation(_) => Kind::Annotation,
        }
    }
    /// Inline-level content placed here lies inside the part.
    fn holds_content(&self) -> bool {
        matches!(self, Self::Base { .. } | Self::Annotation(_))
    }
}

struct PendingAnnotation {
    block: Option<usize>,
    style: Arc<Style>,
    text: String,
    anonymous: bool,
    whitespace: bool,
}

struct PendingLevel {
    style: Arc<Style>,
    annotations: Vec<PendingAnnotation>,
}

/// A ruby container open in an inline formatting context.
struct OpenRuby {
    /// An element's, rather than one wrapping misparented internal ruby boxes.
    explicit: bool,
    style: Arc<Style>,
    container: usize,
    parts: Vec<Part>,
    /// Non-ruby inline boxes open inside the innermost part.
    nesting: usize,
    bases: Vec<(usize, String)>,
    levels: Vec<PendingLevel>,
    segments: Vec<Segment>,
    /// Intra-ruby white space whose next sibling is still to come, and the kind of box before
    /// it in the same container, if any.
    whitespace: Option<(String, Style)>,
    previous: Option<Kind>,
}

/// Where a ruby-internal box of display `display` is properly contained: in the container
/// itself, or in the innermost explicit part of kind `parent` (§2.2).
fn properly_contained(display: &Display, parent: Option<Kind>) -> bool {
    match display {
        Display::RubyBase => matches!(parent, None | Some(Kind::BaseContainer)),
        Display::RubyText => matches!(parent, None | Some(Kind::Level)),
        _ => parent.is_none(),
    }
}

/// The ruby containers open in a block's inline content, innermost last.
#[derive(Default)]
pub(super) struct RubyStack {
    open: Vec<OpenRuby>,
}

/// What closing an inline-level element did to the ruby structure.
#[derive(PartialEq, Eq)]
pub(super) enum Closed {
    /// It was an ordinary inline box, which the caller closes.
    Inline,
    /// It was a ruby box, now closed.
    Ruby,
}

impl RubyStack {
    /// Whether content placed next lies in a ruby annotation.
    pub(super) fn in_annotation(&self) -> bool {
        self.open
            .iter()
            .flat_map(|ruby| &ruby.parts)
            .any(|part| matches!(part, Part::Annotation(_)))
    }

    /// The innermost open annotation's base level among `self.open`, outermost first: where
    /// out-of-flow boxes inside annotations anchor, since their containing block is the ruby
    /// container's (§2.1.1).
    pub(super) fn base_level(&mut self) -> Option<&mut InlineCollector> {
        self.open
            .iter_mut()
            .flat_map(|ruby| ruby.parts.iter_mut())
            .find_map(|part| match part {
                Part::Annotation(annotation) => Some(&mut annotation.outer),
                _ => None,
            })
    }

    /// Text appended next; `true` where it is intra-ruby white space held until its next
    /// sibling classifies it.
    pub(super) fn annotation_mode(&self) -> Option<crate::style::logical::WritingMode> {
        self.open
            .iter()
            .rev()
            .flat_map(|ruby| ruby.parts.iter().rev())
            .find_map(|part| match part {
                Part::Annotation(annotation) => Some(annotation.style.used_writing_mode()),
                _ => None,
            })
    }

    pub(super) fn text(
        &mut self,
        inline: &mut InlineCollector,
        text: &str,
        style: &Style,
    ) -> Result<bool, DocumentError> {
        if let Some(ruby) = self.open.last_mut()
            && ruby.nesting == 0
            && !ruby.parts.last().is_some_and(Part::holds_content)
            && text.chars().all(|ch| {
                crate::document::inline::collapses_character(ch, style.white_space.collapse)
            })
        {
            match &mut ruby.whitespace {
                Some((pending, _)) => pending.push_str(text),
                None => ruby.whitespace = Some((text.to_owned(), style.clone())),
            }
            return Ok(true);
        }
        self.content(inline)?;
        // §2.4: auto-hiding compares the text content of annotations and their bases.
        for part in self.open.iter_mut().flat_map(|ruby| ruby.parts.iter_mut()) {
            match part {
                Part::Base { text: content, .. } => content.push_str(text),
                Part::Annotation(annotation) => annotation.text.push_str(text),
                Part::BaseContainer { .. } | Part::Level { .. } => {}
            }
        }
        Ok(false)
    }

    /// Ensures that inline-level content placed next lies in a ruby base or annotation of an
    /// open ruby container, opening anonymous ones (§2.2), and ending an anonymous container it
    /// does not belong to.
    pub(super) fn content(&mut self, inline: &mut InlineCollector) -> Result<(), DocumentError> {
        loop {
            let Some(ruby) = self.open.last_mut() else {
                return Ok(());
            };
            if ruby.nesting > 0 || ruby.parts.last().is_some_and(Part::holds_content) {
                return Ok(());
            }
            if !ruby.explicit && !ruby.parts.iter().any(Part::explicit) {
                self.end_anonymous(inline)?;
                continue;
            }
            if let Some(Part::Level {
                explicit: true,
                style,
            }) = ruby.parts.last()
            {
                let style = Arc::new(anonymous(style, Display::RubyText));
                ruby.open_annotation(inline, style, false);
            } else {
                ruby.open_base(inline, None)?;
            }
            // White space before the content begins the base or annotation it opens.
            if let Some((text, style)) = ruby.whitespace.take() {
                inline.append(&text, &style);
            }
            return Ok(());
        }
    }

    /// Opens an ordinary inline box, whose contents lie in the innermost ruby part.
    pub(super) fn inline_box(&mut self, inline: &mut InlineCollector) -> Result<(), DocumentError> {
        self.content(inline)?;
        if let Some(ruby) = self.open.last_mut() {
            ruby.nesting += 1;
        }
        Ok(())
    }

    /// Opens the ruby box of an element styled `style`. A ruby-internal box that its parent
    /// does not properly contain opens an anonymous ruby container, which inherits from the
    /// innermost open box or else from `block`.
    pub(super) fn open(
        &mut self,
        inline: &mut InlineCollector,
        style: &Style,
        decoration: BoxDecoration,
        navigation: BoxNavigation,
        block: &Style,
    ) -> Result<(), DocumentError> {
        let display = &style.display;
        if *display == Display::Ruby {
            self.content(inline)?;
            inline.open_box(style, decoration, navigation, false);
            self.open.push(OpenRuby::new(true, style, inline));
            return Ok(());
        }
        let contained = self.open.last().is_some_and(|ruby| {
            ruby.nesting == 0
                && properly_contained(
                    display,
                    ruby.parts
                        .iter()
                        .rfind(|part| part.explicit())
                        .map(Part::kind),
                )
        });
        if !contained {
            self.content(inline)?;
            let anonymous = self.parent_style(inline).map_or_else(
                || anonymous(block, Display::Ruby),
                |parent| anonymous(parent, Display::Ruby),
            );
            inline.open_box(
                &anonymous,
                BoxDecoration::default(),
                BoxNavigation::default(),
                false,
            );
            self.open.push(OpenRuby::new(false, &anonymous, inline));
        }
        let ruby = self
            .open
            .last_mut()
            .expect("a ruby container holds the box");
        let style = Arc::new(style.clone());
        match display {
            Display::RubyBase => {
                ruby.classify(inline, Kind::Base);
                ruby.open_base(inline, Some((style, decoration, navigation)))?;
            }
            Display::RubyText => {
                ruby.classify(inline, Kind::Annotation);
                while ruby
                    .parts
                    .last()
                    .is_some_and(|part| !part.explicit() && part.kind() != Kind::Level)
                {
                    ruby.close_part(inline)?;
                }
                if ruby.parts.is_empty() {
                    let level = Arc::new(level_style(&ruby.style));
                    ruby.levels.push(PendingLevel {
                        style: level.clone(),
                        annotations: Vec::new(),
                    });
                    ruby.parts.push(Part::Level {
                        explicit: false,
                        style: level,
                    });
                }
                let element = (*style).clone();
                ruby.open_annotation(inline, style, true);
                inline.open_box(&element, decoration, navigation, false);
            }
            Display::RubyTextContainer => {
                ruby.classify(inline, Kind::Level);
                ruby.close_anonymous(inline)?;
                ruby.parts.push(Part::Level {
                    explicit: true,
                    style: style.clone(),
                });
                ruby.levels.push(PendingLevel {
                    style,
                    annotations: Vec::new(),
                });
                ruby.previous = None;
            }
            Display::RubyBaseContainer => {
                ruby.classify(inline, Kind::BaseContainer);
                ruby.close_anonymous(inline)?;
                ruby.end_segment(inline);
                ruby.parts.push(Part::BaseContainer {
                    explicit: true,
                    style,
                });
                ruby.previous = None;
            }
            _ => unreachable!("only internal ruby boxes remain"),
        }
        Ok(())
    }

    /// The style of the box that content placed next lies in, where it is open here.
    fn parent_style<'a>(&'a self, inline: &'a InlineCollector) -> Option<&'a Style> {
        inline.open_style().or_else(|| {
            let ruby = self.open.last()?;
            Some(match ruby.parts.last() {
                Some(Part::Annotation(annotation)) => &annotation.style,
                Some(Part::Level { style, .. } | Part::BaseContainer { style, .. }) => style,
                Some(Part::Base { .. }) | None => &ruby.style,
            })
        })
    }

    /// Closes the inline-level element whose contents end here.
    pub(super) fn close(&mut self, inline: &mut InlineCollector) -> Result<Closed, DocumentError> {
        loop {
            let Some(ruby) = self.open.last_mut() else {
                return Ok(Closed::Inline);
            };
            if ruby.nesting > 0 {
                ruby.nesting -= 1;
                return Ok(Closed::Inline);
            }
            if ruby.parts.iter().any(Part::explicit) {
                ruby.whitespace = None;
                while ruby.parts.last().is_some_and(|part| !part.explicit()) {
                    ruby.close_part(inline)?;
                }
                ruby.close_part(inline)?;
                return Ok(Closed::Ruby);
            }
            let explicit = ruby.explicit;
            if explicit {
                ruby.whitespace = None;
                let ruby = self.open.pop().expect("the ruby container closes");
                ruby.finish(inline)?;
                return Ok(Closed::Ruby);
            }
            // An anonymous container ends with its parent, which closes next.
            self.end_anonymous(inline)?;
        }
    }

    /// Closes the anonymous ruby containers left open at the end of inline content.
    pub(super) fn finish(&mut self, inline: &mut InlineCollector) -> Result<(), DocumentError> {
        while self.open.last().is_some_and(|ruby| !ruby.explicit) {
            self.end_anonymous(inline)?;
        }
        Ok(())
    }

    /// Ends the innermost, anonymous container: white space after its last box follows it.
    fn end_anonymous(&mut self, inline: &mut InlineCollector) -> Result<(), DocumentError> {
        let mut ruby = self.open.pop().expect("an anonymous container is open");
        let trailing = ruby.whitespace.take();
        ruby.finish(inline)?;
        if let Some((text, style)) = trailing
            && !self.text(inline, &text, &style)?
        {
            inline.append(&text, &style);
        }
        Ok(())
    }
}

/// An anonymous ruby box inheriting from `parent`, isolated as ruby boxes are (§3.5).
pub(super) fn anonymous(parent: &Style, display: Display) -> Style {
    let mut style = parent.anonymous(display);
    style.unicode_bidi = style.unicode_bidi.forced_isolation();
    style
}

/// The anonymous annotation container of `container`, which receives no decorations beyond
/// its own (§4.4).
pub(super) fn level_style(container: &Style) -> Style {
    let mut style = anonymous(container, Display::RubyTextContainer);
    style.text_decorations.propagated.clear();
    style
}

impl OpenRuby {
    fn new(explicit: bool, style: &Style, inline: &InlineCollector) -> Self {
        Self {
            explicit,
            style: Arc::new(style.clone()),
            container: inline
                .innermost_box()
                .expect("the container's inline box was just opened"),
            parts: Vec::new(),
            nesting: 0,
            bases: Vec::new(),
            levels: Vec::new(),
            segments: Vec::new(),
            whitespace: None,
            previous: None,
        }
    }

    /// The innermost annotation container's style.
    fn level_style(&self) -> Arc<Style> {
        self.parts
            .iter()
            .rev()
            .find_map(|part| match part {
                Part::Level { style, .. } => Some(style.clone()),
                _ => None,
            })
            .expect("annotations lie in an annotation container")
    }

    /// Classifies pending intra-ruby white space before a box of kind `next` (§2.2): leading,
    /// inter-level and other discarded white space goes, inter-base and inter-segment white
    /// space joins the base level, and inter-annotation white space its level.
    fn classify(&mut self, inline: &mut InlineCollector, next: Kind) {
        let Some((text, style)) = self.whitespace.take() else {
            return;
        };
        match (self.previous, next) {
            (None, _) | (_, Kind::Level) => {}
            (Some(Kind::Annotation), Kind::Annotation) => {
                let style = Arc::new(style);
                if let Some(level) = self.levels.last_mut() {
                    level.annotations.push(PendingAnnotation {
                        block: None,
                        style,
                        text,
                        anonymous: true,
                        whitespace: true,
                    });
                }
            }
            (Some(_), Kind::Annotation) => {}
            (Some(previous), Kind::Base | Kind::BaseContainer) => {
                if matches!(previous, Kind::Annotation | Kind::Level) {
                    self.close_anonymous_level();
                    self.end_segment(inline);
                }
                inline.append(&text, &style);
            }
        }
    }

    /// Closes an anonymous annotation container that the next base ends.
    fn close_anonymous_level(&mut self) {
        if matches!(
            self.parts.last(),
            Some(Part::Level {
                explicit: false,
                ..
            })
        ) {
            self.parts.pop();
        }
    }

    /// Closes the anonymous parts that a new base or annotation container ends.
    fn close_anonymous(&mut self, inline: &mut InlineCollector) -> Result<(), DocumentError> {
        while self.parts.last().is_some_and(|part| !part.explicit()) {
            self.close_part(inline)?;
        }
        Ok(())
    }

    /// Opens a ruby base, explicit with its element's style, decoration and navigation, or
    /// anonymous, in a base container of the current segment.
    fn open_base(
        &mut self,
        inline: &mut InlineCollector,
        element: Option<(Arc<Style>, BoxDecoration, BoxNavigation)>,
    ) -> Result<(), DocumentError> {
        while self
            .parts
            .last()
            .is_some_and(|part| !part.explicit() && part.kind() != Kind::BaseContainer)
        {
            self.close_part(inline)?;
        }
        if !matches!(self.parts.last(), Some(Part::BaseContainer { .. })) {
            self.end_segment(inline);
            self.parts.push(Part::BaseContainer {
                explicit: false,
                style: self.style.clone(),
            });
        }
        let explicit = element.is_some();
        if let Some((style, decoration, navigation)) = element {
            inline.open_box(&style, decoration, navigation, false);
        } else {
            let Some(Part::BaseContainer { style, .. }) = self.parts.last() else {
                unreachable!("a base container was just ensured");
            };
            inline.open_box(
                &anonymous(style, Display::RubyBase),
                BoxDecoration::default(),
                BoxNavigation::default(),
                false,
            );
        }
        self.parts.push(Part::Base {
            explicit,
            inline_box: inline.innermost_box().expect("the base was just opened"),
            text: String::new(),
        });
        Ok(())
    }

    /// Sets the base level aside to collect an annotation styled `style`.
    fn open_annotation(
        &mut self,
        inline: &mut InlineCollector,
        mut style: Arc<Style>,
        explicit: bool,
    ) {
        if style.ruby.position == RubyPosition::InterCharacter
            && !self.style.used_writing_mode().vertical()
        {
            Arc::make_mut(&mut style).writing_mode = crate::style::logical::WritingMode::VerticalRl;
        }
        self.parts.push(Part::Annotation(Box::new(OpenAnnotation {
            explicit,
            outer: std::mem::take(inline),
            style,
            text: String::new(),
        })));
    }

    /// Closes the innermost part.
    fn close_part(&mut self, inline: &mut InlineCollector) -> Result<(), DocumentError> {
        let part = self.parts.pop().expect("a ruby part is open");
        let kind = part.kind();
        // Anonymous containers are not siblings of the white space inside them.
        let sibling = part.explicit() || part.holds_content();
        match part {
            Part::Base {
                inline_box, text, ..
            } => {
                inline.close_box();
                self.bases.push((inline_box, text));
            }
            Part::Annotation(annotation) => {
                let OpenAnnotation {
                    explicit,
                    outer,
                    style,
                    text,
                } = *annotation;
                if explicit {
                    inline.close_box();
                }
                let inter_character = style.ruby.position == RubyPosition::InterCharacter
                    && !self.style.used_writing_mode().vertical();
                let wrapper = annotation_wrapper(&self.level_style(), &style, inter_character);
                let contents = std::mem::replace(inline, outer);
                // The first formatted line holding the annotation restyles its contents
                // before `text-transform` (CSS Pseudo-Elements 4 §2.1.3).
                let mut source = None;
                let finished = contents.finish_with_first_line(&wrapper, |contents| {
                    source = Some(contents.untransformed_copy());
                    Ok(None)
                })?;
                let block = match finished {
                    Ok(mut paragraph) => {
                        if inter_character {
                            paragraph.align_annotation(style.ruby.align);
                        }
                        let mut block = BlockBox::anonymous(
                            wrapper,
                            vec![BlockContent::Inline(paragraph)],
                            None,
                        );
                        if let Some(mut source) = source {
                            if inter_character {
                                source.align_annotation(style.ruby.align);
                            }
                            block.keep_annotation_source(source);
                        }
                        Some(inline.push_annotation(block))
                    }
                    Err(stray) => {
                        for content in stray {
                            match content {
                                BlockContent::Events(events) => {
                                    for event in events {
                                        inline.semantic_event(event);
                                    }
                                }
                                BlockContent::Positioned(block) => inline.anchor(block),
                                BlockContent::Block(_) | BlockContent::Inline(_) => {
                                    unreachable!("empty inline content holds no flow boxes")
                                }
                            }
                        }
                        None
                    }
                };
                self.levels
                    .last_mut()
                    .expect("annotations lie in an annotation container")
                    .annotations
                    .push(PendingAnnotation {
                        block,
                        style,
                        text,
                        anonymous: !explicit,
                        whitespace: false,
                    });
            }
            Part::BaseContainer { .. } | Part::Level { .. } => {}
        }
        if sibling {
            self.previous = Some(kind);
        }
        Ok(())
    }

    /// Ends the current segment, pairing its annotations with its bases (§2.3.2) and adding
    /// empty anonymous bases at the end of its base container for annotations beyond them.
    fn end_segment(&mut self, inline: &mut InlineCollector) {
        if self.bases.is_empty() && self.levels.is_empty() {
            return;
        }
        let needed = self
            .levels
            .iter()
            .map(|level| match pairing(level) {
                Pairing::Spanning => 1,
                Pairing::Each(count) => count,
            })
            .max()
            .unwrap_or(0);
        while self.bases.len() < needed {
            let base = anonymous(&self.style, Display::RubyBase);
            inline.open_box(
                &base,
                BoxDecoration::default(),
                BoxNavigation::default(),
                false,
            );
            let index = inline
                .innermost_box()
                .expect("the empty base was just opened");
            inline.close_box();
            self.bases.push((index, String::new()));
        }
        let count = self.bases.len();
        let mut levels = Vec::with_capacity(self.levels.len());
        let mut previous = None;
        for level in std::mem::take(&mut self.levels) {
            let inter_character = level.style.ruby.position == RubyPosition::InterCharacter
                && !self.style.used_writing_mode().vertical();
            let under = level.style.ruby.position.under(previous);
            if !inter_character {
                previous = Some(under);
            }
            let spanning = pairing(&level) == Pairing::Spanning;
            let hides = level.style.ruby.merge != RubyMerge::Merge;
            let mut seen = 0;
            let mut annotations = Vec::with_capacity(level.annotations.len());
            for annotation in level.annotations {
                let bases = if annotation.whitespace {
                    seen..seen
                } else if spanning {
                    0..count
                } else {
                    seen += 1;
                    seen - 1..seen
                };
                let text: String = self.bases[bases.clone()]
                    .iter()
                    .map(|(_, text)| text.as_str())
                    .collect();
                annotations.push(Annotation {
                    hidden: !annotation.whitespace
                        && (annotation.style.visibility == Visibility::Collapse
                            || (hides && !bases.is_empty() && annotation.text == text)),
                    block: annotation.block,
                    style: annotation.style,
                    bases,
                    whitespace: annotation.whitespace,
                });
            }
            levels.push(Level {
                style: level.style,
                under,
                inter_character,
                annotations,
            });
        }
        self.segments.push(Segment {
            bases: std::mem::take(&mut self.bases)
                .into_iter()
                .map(|(index, _)| index)
                .collect(),
            levels,
        });
    }

    /// Closes the container: its last segment ends, and the paragraph records its structure.
    fn finish(mut self, inline: &mut InlineCollector) -> Result<(), DocumentError> {
        while !self.parts.is_empty() {
            self.close_part(inline)?;
        }
        self.end_segment(inline);
        inline.close_box();
        if !self.segments.is_empty() {
            inline.push_ruby(Ruby {
                container: self.container,
                segments: self.segments,
            });
        }
        Ok(())
    }
}

/// How a level's annotations pair with the bases of their segment (§2.3.2).
#[derive(PartialEq, Eq)]
enum Pairing {
    /// A lone anonymous annotation spans every base.
    Spanning,
    /// Each annotation pairs with one base, in order; this many need a base.
    Each(usize),
}

fn pairing(level: &PendingLevel) -> Pairing {
    let real: Vec<&PendingAnnotation> = level
        .annotations
        .iter()
        .filter(|annotation| !annotation.whitespace)
        .collect();
    match real.as_slice() {
        [single] if single.anonymous => Pairing::Spanning,
        real => Pairing::Each(real.len()),
    }
}

/// The anonymous box an annotation of the level styled `level` is laid out in: its
/// annotation-level formatting context, which keeps its contents on one line at their
/// max-content width and distributes any extra width it is given (§§3.1, 4.3).
/// The style of the block that lays out the contents of an annotation box styled `annotation`
/// in an annotation container styled `level`; an inter-character annotation's is the box's,
/// vertical (§2.1.1, §4.1).
pub(super) fn annotation_wrapper(
    level: &Style,
    annotation: &Style,
    inter_character: bool,
) -> Style {
    if inter_character {
        let mut wrapper = wrapper_style(annotation);
        wrapper.writing_mode = crate::style::logical::WritingMode::VerticalRl;
        wrapper
    } else {
        wrapper_style(level)
    }
}

fn wrapper_style(level: &Style) -> Style {
    let mut style = level.anonymous(Display::InlineBlock);
    style.width = crate::style::Size::Intrinsic(crate::style::IntrinsicSizing::MaxContent);
    style.align = TextAlign::Justify;
    style.align_last = Some(TextAlign::Justify);
    style.text_indent = crate::style::TextIndent::default();
    style
}
