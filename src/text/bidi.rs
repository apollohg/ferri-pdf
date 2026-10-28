use crate::document::{MAX_INLINE_TEXT_BYTES, Paragraph};
use crate::text::TextError;
use ferri_pdf_style::{Style, UnicodeBidi};
use std::{cell::Cell, ops::Range};
use unicode_bidi::{BidiClass, BidiInfo, Level, ParagraphBidiInfo, ParagraphInfo, bidi_class};

const LRE: &str = "\u{202a}";
const RLE: &str = "\u{202b}";
const PDF: &str = "\u{202c}";
const LRO: &str = "\u{202d}";
const RLO: &str = "\u{202e}";
const LRI: &str = "\u{2066}";
const RLI: &str = "\u{2067}";
const FSI: &str = "\u{2068}";
const PDI: &str = "\u{2069}";
const FSI_LRO: &str = "\u{2068}\u{202d}";
const FSI_RLO: &str = "\u{2068}\u{202e}";
const PDF_PDI: &str = "\u{202c}\u{2069}";

type Controls = (&'static str, &'static str);

#[derive(Clone, Copy, Debug)]
pub(super) struct ExpansionLimit;
impl From<ExpansionLimit> for TextError {
    fn from(_: ExpansionLimit) -> Self {
        Self::BidiExpansionLimit {
            limit: MAX_INLINE_TEXT_BYTES,
        }
    }
}

fn controls(style: &Style, root: bool) -> Controls {
    match style.unicode_bidi {
        UnicodeBidi::Normal => ("", ""),
        UnicodeBidi::Embed if !root => (if style.rtl { RLE } else { LRE }, PDF),
        UnicodeBidi::Isolate if !root => (if style.rtl { RLI } else { LRI }, PDI),
        UnicodeBidi::BidiOverride => (if style.rtl { RLO } else { LRO }, PDF),
        UnicodeBidi::IsolateOverride if !root => {
            (if style.rtl { FSI_RLO } else { FSI_LRO }, PDF_PDI)
        }
        UnicodeBidi::IsolateOverride => (if style.rtl { RLO } else { LRO }, PDF),
        UnicodeBidi::Plaintext if !root => (FSI, PDI),
        _ => ("", ""),
    }
}

struct Event {
    offset: usize,
    controls: Controls,
    opening: bool,
}

fn events(paragraph: &Paragraph) -> Vec<Event> {
    let mut result = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let boxes = paragraph.boxes();
    let close = |index: usize, events: &mut Vec<Event>| {
        let item = &boxes[index];
        let pair = controls(&item.style, false);
        if !pair.0.is_empty() {
            events.push(Event {
                offset: item.range.end,
                controls: pair,
                opening: false,
            });
        }
    };
    for (index, item) in boxes.iter().enumerate() {
        while stack.last().copied() != item.parent {
            close(
                stack.pop().expect("inline box ancestry is in tree order"),
                &mut result,
            );
        }
        let pair = controls(&item.style, false);
        if !pair.0.is_empty() {
            result.push(Event {
                offset: item.range.start,
                controls: pair,
                opening: true,
            });
        }
        stack.push(index);
    }
    for index in stack.into_iter().rev() {
        close(index, &mut result);
    }
    result
}

pub(super) enum ResolvedParagraph<'p> {
    Plain(BidiInfo<'p>),
    Controlled(Controlled),
}
pub(super) struct Controlled {
    source_levels: Vec<Level>,
    text: String,
    classes: Vec<BidiClass>,
    levels: Vec<Level>,
    paragraphs: Vec<ParagraphInfo>,
    chunks: Vec<SourceChunk>,
}

fn reorder(
    text: &str,
    classes: &[BidiClass],
    levels: &[Level],
    paragraphs: &[ParagraphInfo],
    range: Range<usize>,
) -> Vec<(Range<usize>, bool)> {
    let paragraph =
        &paragraphs[paragraphs.partition_point(|paragraph| paragraph.range.end <= range.start)];
    let line = ParagraphBidiInfo {
        text: &text[range.clone()],
        original_classes: classes[range.clone()].to_vec(),
        levels: levels[range.clone()].to_vec(),
        paragraph_level: paragraph.level,
        is_pure_ltr: false,
    };
    let (levels, visual) = line.visual_runs(0..range.len());
    visual
        .into_iter()
        .map(|run| {
            let rtl = levels[run.start].is_rtl();
            (range.start + run.start..range.start + run.end, rtl)
        })
        .collect()
}
impl ResolvedParagraph<'_> {
    pub(super) fn has_controls(&self) -> bool {
        matches!(self, Self::Controlled(_))
    }
    pub(super) fn level(&self, offset: usize) -> Level {
        match self {
            Self::Plain(source) => source.levels[offset],
            Self::Controlled(controlled) => controlled.source_levels[offset],
        }
    }
    pub(super) fn has_rtl(&self) -> bool {
        match self {
            Self::Plain(source) => source.has_rtl(),
            Self::Controlled(controlled) => controlled.levels.iter().any(Level::is_rtl),
        }
    }
    pub(super) fn visual_runs(&self, range: Range<usize>) -> Vec<(Range<usize>, bool)> {
        let controlled = match self {
            Self::Plain(source) => {
                return reorder(
                    source.text,
                    &source.original_classes,
                    &source.levels,
                    &source.paragraphs,
                    range,
                );
            }
            Self::Controlled(controlled) => controlled,
        };
        let first = controlled
            .chunks
            .partition_point(|chunk| chunk.source.end <= range.start);
        let last = controlled
            .chunks
            .partition_point(|chunk| chunk.source.start < range.end)
            - 1;
        let left = &controlled.chunks[first];
        let right = &controlled.chunks[last];
        let virtual_range = left.virtual_start + range.start - left.source.start
            ..right.virtual_start + range.end - right.source.start;
        let mut result = Vec::new();
        for (run, rtl) in reorder(
            &controlled.text,
            &controlled.classes,
            &controlled.levels,
            &controlled.paragraphs,
            virtual_range,
        ) {
            let first = controlled
                .chunks
                .partition_point(|chunk| chunk.virtual_start + chunk.source.len() <= run.start);
            let mut pieces = Vec::new();
            for chunk in controlled.chunks[first..]
                .iter()
                .take_while(|chunk| chunk.virtual_start < run.end)
            {
                let start = run.start.max(chunk.virtual_start);
                let end = run.end.min(chunk.virtual_start + chunk.source.len());
                if start < end {
                    pieces.push((
                        chunk.source.start + start - chunk.virtual_start
                            ..chunk.source.start + end - chunk.virtual_start,
                        rtl,
                    ));
                }
            }
            if rtl {
                pieces.reverse();
            }
            result.extend(pieces);
        }
        result
    }
}

struct SourceChunk {
    source: Range<usize>,
    virtual_start: usize,
}

struct Stream<'budget> {
    work: &'budget Cell<Option<usize>>,
    text: String,
    chunks: Vec<SourceChunk>,
    active: Vec<Controls>,
    opened: bool,
}
impl<'budget> Stream<'budget> {
    fn new(work: &'budget Cell<Option<usize>>) -> Self {
        Self {
            work,
            text: String::new(),
            chunks: Vec::new(),
            active: Vec::new(),
            opened: false,
        }
    }
    fn push(&mut self, text: &str) -> Result<(), ExpansionLimit> {
        let remaining = self
            .work
            .get()
            .and_then(|work| work.checked_sub(text.len()));
        self.work.set(remaining);
        if remaining.is_none()
            || self
                .text
                .len()
                .checked_add(text.len())
                .is_none_or(|size| size > MAX_INLINE_TEXT_BYTES)
        {
            self.work.set(None);
            return Err(ExpansionLimit);
        }
        self.text.push_str(text);
        Ok(())
    }
    fn open(&mut self) -> Result<(), ExpansionLimit> {
        if !self.opened {
            for index in 0..self.active.len() {
                self.push(self.active[index].0)?;
            }
            self.opened = true;
        }
        Ok(())
    }
    fn close(&mut self) -> Result<(), ExpansionLimit> {
        if self.opened {
            for index in (0..self.active.len()).rev() {
                self.push(self.active[index].1)?;
            }
            self.opened = false;
        }
        Ok(())
    }
    fn source(&mut self, text: &str, range: Range<usize>) -> Result<(), ExpansionLimit> {
        if range.is_empty() {
            return Ok(());
        }
        let virtual_start = self.text.len();
        self.push(&text[range.clone()])?;
        if let Some(last) = self.chunks.last_mut()
            && last.source.end == range.start
            && last.virtual_start + last.source.len() == virtual_start
        {
            last.source.end = range.end;
        } else {
            self.chunks.push(SourceChunk {
                source: range,
                virtual_start,
            });
        }
        Ok(())
    }
    fn append(&mut self, text: &str, range: Range<usize>) -> Result<(), ExpansionLimit> {
        let mut from = range.start;
        for (offset, ch) in text[range.clone()].char_indices() {
            if bidi_class(ch) == BidiClass::B {
                let at = range.start + offset;
                if from != at {
                    self.open()?;
                    self.source(text, from..at)?;
                }
                self.close()?;
                self.source(text, at..at + ch.len_utf8())?;
                from = at + ch.len_utf8();
            }
        }
        if from < range.end {
            self.open()?;
            self.source(text, from..range.end)?;
        }
        Ok(())
    }
    fn project(self, text: &str, base: Option<Level>) -> ResolvedParagraph<'_> {
        let resolved = BidiInfo::new(&self.text, base);
        let mut source_levels = Vec::with_capacity(text.len());
        for chunk in &self.chunks {
            source_levels.extend_from_slice(
                &resolved.levels[chunk.virtual_start..chunk.virtual_start + chunk.source.len()],
            );
        }
        let BidiInfo {
            original_classes,
            levels,
            paragraphs,
            ..
        } = resolved;
        ResolvedParagraph::Controlled(Controlled {
            source_levels,
            text: self.text,
            classes: original_classes,
            levels,
            paragraphs,
            chunks: self.chunks,
        })
    }
}

pub(super) fn resolve<'p>(
    paragraph: &'p Paragraph,
    work: &Cell<Option<usize>>,
) -> Result<ResolvedParagraph<'p>, ExpansionLimit> {
    if work.get().is_none() {
        return Err(ExpansionLimit);
    }
    let text = paragraph.text();
    let style = paragraph.style();
    let base = (style.unicode_bidi != UnicodeBidi::Plaintext).then(|| {
        if style.rtl {
            Level::rtl()
        } else {
            Level::ltr()
        }
    });
    let root = controls(style, true);
    if text.is_empty()
        || (root.0.is_empty()
            && paragraph
                .boxes()
                .iter()
                .all(|item| controls(&item.style, false).0.is_empty()))
    {
        return Ok(ResolvedParagraph::Plain(BidiInfo::new(text, base)));
    }
    let events = events(paragraph);
    let mut stream = Stream::new(work);
    if !root.0.is_empty() {
        stream.active.push(root);
    }
    let mut from = 0;
    for event in events {
        stream.append(text, from..event.offset)?;
        if event.opening {
            stream.open()?;
            stream.push(event.controls.0)?;
            stream.active.push(event.controls);
        } else {
            if stream.opened {
                stream.push(event.controls.1)?;
            }
            stream.active.pop();
        }
        from = event.offset;
    }
    stream.append(text, from..text.len())?;
    stream.close()?;
    Ok(stream.project(text, base))
}
