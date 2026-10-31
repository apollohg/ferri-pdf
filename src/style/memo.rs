//! Computed styles kept for formatting the document again with the same inputs.
use crate::dom::ElementId;
use crate::style::{
    GeneratedContent, PageGeometry, PageRequest, PseudoElement, Style, StyleError, Stylesheet,
    declaration::{Declared, Specified},
    grammar::Property,
};
use std::{cell::Cell, collections::HashMap, hash::Hash, rc::Rc};

/// The styles a stylesheet computed, by element or pseudo-element and the parent style each
/// inherited from. Besides those, a computation depends only on the root's style, the first
/// page and its geometry, the container sizes and anchor frames the last layout measured and
/// the scroll markers' states, whose changes forget them; so an equal parent computes an equal
/// style.
#[derive(Default)]
pub(crate) struct StyleMemo {
    /// Boxed: each frame that holds the document by value under a deep layout holds this.
    context: Option<Box<Context>>,
    pub(super) kept: KeptStyles,
    /// Set once content shows a value that pagination gives, which makes formatting the
    /// document again likely: only then are styles kept, from there on.
    keeping: Rc<Cell<bool>>,
}

/// The root's style and the first page the formatting computing styles started with.
#[derive(PartialEq)]
struct Context {
    root: Style,
    first_page: PageRequest,
    progression_rtl: bool,
    geometry: PageGeometry,
}

#[derive(Default)]
pub(super) struct KeptStyles {
    pub(super) elements: Kept<(ElementId, bool), Style>,
    pub(super) pseudos: Kept<(ElementId, PseudoElement), Option<(Style, GeneratedContent)>>,
    pub(super) typographic: Kept<(ElementId, PseudoElement), Style>,
    /// The parent last kept, which its children's and pseudo-elements' styles share.
    parent: Option<Rc<Style>>,
}

/// Results of one kind of computation, with the parents they inherited from.
pub(super) struct Kept<K, V: Computed>(HashMap<K, Vec<Inherited<V>>>);
type Inherited<V> = (Rc<Style>, <V as Computed>::Kept);
impl<K, V: Computed> Default for Kept<K, V> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}
impl<K: Eq + Hash, V: Computed> Kept<K, V> {
    fn get(&self, key: &K, parent: &Style) -> Option<V> {
        self.0
            .get(key)?
            .iter()
            .find(|(kept, _)| same_parent(kept, parent))
            .map(|(_, value)| V::revived(value))
    }
}

/// Whether styles computed with `kept` and `parent` as their parent are equal: the query
/// containers a style holds are context its equality ignores.
fn same_parent(kept: &Style, parent: &Style) -> bool {
    kept == parent && kept.containers.same(&parent.containers)
}

/// A computation's result and the form it is kept in. Styles with position options or page
/// variants are not kept: these hold origins of their own, which a later computation's would
/// not share.
pub(super) trait Computed: Sized {
    type Kept;
    fn kept(&self) -> Option<Self::Kept>;
    fn revived(kept: &Self::Kept) -> Self;
}
fn kept_style(style: &Style) -> Option<Rc<Style>> {
    (style.anchor.pages.is_empty() && style.anchor.options.is_empty())
        .then(|| Rc::new(style.clone()))
}
impl Computed for Style {
    type Kept = Rc<Style>;
    fn kept(&self) -> Option<Self::Kept> {
        kept_style(self)
    }
    fn revived(kept: &Self::Kept) -> Self {
        Self::clone(kept)
    }
}
impl Computed for Option<(Style, GeneratedContent)> {
    type Kept = Option<(Rc<Style>, GeneratedContent)>;
    fn kept(&self) -> Option<Self::Kept> {
        match self {
            None => Some(None),
            Some((style, content)) => kept_style(style).map(|style| Some((style, content.clone()))),
        }
    }
    fn revived(kept: &Self::Kept) -> Self {
        kept.as_ref()
            .map(|(style, content)| (Style::clone(style), content.clone()))
    }
}

impl StyleMemo {
    pub(super) fn forget(&mut self) {
        self.kept = KeptStyles::default();
    }
}

impl KeptStyles {
    /// `parent` as kept: the parent last kept or the kept style of its element where equal.
    fn shared(&self, parent: &Style) -> Rc<Style> {
        if let Some(last) = self
            .parent
            .as_ref()
            .filter(|last| same_parent(last, parent))
        {
            return last.clone();
        }
        parent
            .origin
            .node()
            .filter(|origin| origin.pseudo.is_none())
            .and_then(|origin| {
                [false, true]
                    .into_iter()
                    .filter_map(|without_box| self.elements.0.get(&(origin.element, without_box)))
                    .flatten()
                    .map(|(_, style)| style)
                    .find(|style| same_parent(style, parent))
                    .cloned()
            })
            .unwrap_or_else(|| Rc::new(parent.clone()))
    }
}

impl Stylesheet {
    /// Starts a formatting whose root has `root` style, forgetting the styles kept where it or
    /// the first page changed.
    pub(crate) fn start_formatting(&mut self, root: &Style) {
        let context = Context {
            root: root.clone(),
            first_page: self.first_page.clone(),
            progression_rtl: self.page_progression_rtl,
            geometry: self.page_geometry(),
        };
        let memo = self.memo.get_mut();
        if memo.context.as_deref() != Some(&context) {
            memo.context = Some(Box::new(context));
            memo.forget();
        }
    }

    /// Forgets the styles computed so far, for a change to what they depend on outside the
    /// stylesheet.
    pub(crate) fn forget_computed(&self) {
        self.memo.borrow_mut().forget();
    }

    /// The switch that content showing a value that pagination gives turns on, from when
    /// styles are kept.
    pub(crate) fn keeping_computed(&self) -> Rc<Cell<bool>> {
        self.memo.borrow().keeping.clone()
    }

    /// Keeps styles from the first formatting on where a rule declares content that can show
    /// a value that pagination gives, or content whose value only computing it tells.
    pub(super) fn keep_for_paged_content(&mut self) {
        let paged = self
            .rules
            .iter()
            .flat_map(|rule| &rule.declarations)
            .any(|declaration| match &declaration.declared {
                Declared::Longhand(Property::Content, Specified::Value(value)) => {
                    super::generated::shows_paged_values(value)
                }
                Declared::Pending(Property::Content, _) => true,
                _ => false,
            });
        self.memo.get_mut().keeping.set(paged);
    }

    /// What `compute` computes for `key` with `parent`, or what an earlier computation with
    /// an equal parent kept, which `revisit` makes this computation's own.
    pub(super) fn memoized<K: Eq + Hash, V: Computed>(
        &self,
        kept: fn(&mut KeptStyles) -> &mut Kept<K, V>,
        key: K,
        parent: &Style,
        compute: impl FnOnce() -> Result<V, StyleError>,
        revisit: impl FnOnce(V) -> V,
    ) -> Result<V, StyleError> {
        let found = kept(&mut self.memo.borrow_mut().kept).get(&key, parent);
        if let Some(value) = found {
            return Ok(revisit(value));
        }
        crate::work::count(crate::work::Work::ComputedStyle, 1);
        let value = compute()?;
        let mut memo = self.memo.borrow_mut();
        if memo.keeping.get()
            && let Some(stored) = value.kept()
        {
            let parent = memo.kept.shared(parent);
            memo.kept.parent = Some(parent.clone());
            kept(&mut memo.kept)
                .0
                .entry(key)
                .or_default()
                .push((parent, stored));
        }
        Ok(value)
    }
}
