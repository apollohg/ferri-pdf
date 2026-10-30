//! First-page dependencies are evaluated on demand, independently for each physical axis.
use crate::dom::{DocumentTree, Element, TreeRelation};
use crate::fonts::SizeMetric;
use crate::resources::ResourceResolver;
use crate::style::{
    Display, LineHeight, Size, SourceLocation, Style, StyleError, Stylesheet,
    cascade::{Cascaded, Customs, Properties, Winners},
    component::Component,
    computed::{FONT_ADJUSTMENT, FONT_SELECTION},
    conditional::{self, QueryValues},
    declaration::Specified,
    grammar::{Descriptor, Keyword, Property},
    logical::{self, Flow, Flows},
    properties,
    registration::{FontDependencies, Registration},
    stylesheet::PAGE_MARGINS,
    units::{Axis, FontBasis, FontUnit, FontUnits, Relative, Tree, Viewport, ViewportUnits},
    variables::{self, Reference},
    wide_keywords::{Resolved, WideKeyword},
};
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

const PADDING: [Property; 4] = [
    Property::PaddingTop,
    Property::PaddingRight,
    Property::PaddingBottom,
    Property::PaddingLeft,
];
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct PageGeometry {
    pub paper: [f32; 2],
    pub extent: [f32; 2],
    pub area: [f32; 2],
    pub margin: [f32; 4],
    pub padding: [f32; 4],
    pub border: [f32; 4],
}
impl PageGeometry {
    pub fn margin_box(&self) -> crate::paint::Rect {
        crate::paint::Rect {
            x: 0.0,
            y: 0.0,
            width: self.extent[0],
            height: self.extent[1],
        }
    }
    pub fn content_box(&self) -> crate::paint::Rect {
        let edges: [f32; 4] =
            std::array::from_fn(|side| self.margin[side] + self.padding[side] + self.border[side]);
        crate::paint::Rect {
            x: edges[3],
            y: edges[0],
            width: self.area[0],
            height: self.area[1],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Context {
    Root,
    Page,
    Element,
    Tree(crate::style::PageStyleTarget),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Node {
    FirstPageStart,
    FirstPageSide,
    Property(Context, Property),
    Custom(Context, usize),
    Paper(Axis),
    Area(Axis),
}
impl Node {
    fn name(self, geometry: &Geometry<'_>) -> String {
        match self {
            Self::FirstPageStart => "first-page-start".into(),
            Self::FirstPageSide => "first-page-side".into(),
            Self::Property(context, property) => format!("{context:?}.{}", property.name()),
            Self::Custom(context, index) => format!(
                "{context:?}.{}",
                geometry.context(context).customs.names[index]
            ),
            Self::Paper(axis) => format!("page-box.{axis:?}"),
            Self::Area(axis) => format!("first-page-area.{axis:?}"),
        }
    }
}

struct Computed {
    base: Style,
    style: RefCell<Style>,
    properties: Properties,
    customs: Customs,
    variables: variables::Resolver,
    winners: RefCell<Winners>,
    sources: RefCell<HashMap<Property, SourceLocation>>,
    /// Registered custom properties resolving font-relative lengths, and the font properties
    /// they depend on (CSS Properties and Values API 1 §2.7.2).
    font_pending: RefCell<HashMap<String, FontDependencies>>,
    /// Members of font-relative dependency cycles, which are invalid at computed-value time.
    cyclic_customs: RefCell<HashSet<String>>,
    cyclic_properties: RefCell<HashSet<Property>>,
}
impl Computed {
    fn new(style: Style, values: Cascaded) -> Result<Self, StyleError> {
        let (properties, custom) = values.into_parts();
        let customs = Customs::new(custom);
        let variables =
            variables::Resolver::new(&style.inherited_custom_properties, customs.sources()?)?;
        Ok(Self {
            base: style.clone(),
            style: RefCell::new(style),
            properties,
            customs,
            variables,
            winners: RefCell::default(),
            sources: RefCell::default(),
            font_pending: RefCell::default(),
            cyclic_customs: RefCell::default(),
            cyclic_properties: RefCell::default(),
        })
    }
    fn provided(style: Style) -> Result<Self, StyleError> {
        Self::new(style, Cascaded::default())
    }
}

struct Geometry<'a> {
    sheet: &'a Stylesheet,
    page_rules: OnceCell<crate::style::page_rules::MatchedPageRules>,
    viewport: Option<[f32; 2]>,
    resolver: &'a dyn ResourceResolver,
    root: Rc<Computed>,
    page: OnceCell<Rc<Computed>>,
    document: Option<&'a DocumentTree>,
    select_first: Option<&'a crate::style::page_selection::SelectFirstPageName<'a>>,
    select_side: Option<&'a crate::style::page_selection::SelectFirstPageSide<'a>>,
    first_request: RefCell<crate::style::PageRequest>,
    selection_done: Cell<bool>,
    side_selection_done: Cell<bool>,
    tree_contexts: RefCell<HashMap<crate::style::PageStyleTarget, Rc<Computed>>>,
    provided_root: bool,
    provided_element: bool,
    size: OnceCell<Option<Arc<Component>>>,
    size_source: RefCell<Option<SourceLocation>>,
    element: Option<Rc<Computed>>,
    parent: Option<&'a Style>,
    tree: Tree<'a>,
    page_font: bool,
    work: Rc<crate::style::calc::Work>,
    complete: RefCell<HashSet<Node>>,
    dimensions: RefCell<HashMap<Node, f32>>,
    active: RefCell<Vec<Node>>,
    axes: RefCell<[Option<crate::style::UsedAxis>; 2]>,
}

impl<'a> Geometry<'a> {
    fn tree_element(&self, target: crate::style::PageStyleTarget) -> Element<'a> {
        self.document
            .and_then(|document| document.element(target.element))
            .expect("selection targets belong to the document")
    }

    fn target_context(&self, target: crate::style::PageStyleTarget) -> Context {
        if target.pseudo.is_none()
            && self
                .document
                .is_some_and(|document| document.root_element().id() == target.element)
        {
            Context::Root
        } else {
            Context::Tree(target)
        }
    }

    fn parent_context(&self, context: Context) -> Option<Context> {
        match context {
            Context::Page => Some(Context::Root),
            Context::Tree(target) => {
                let element = self.tree_element(target);
                let parent = if target.pseudo.is_some() {
                    Some(element)
                } else {
                    element.parent(TreeRelation::Inheritance)
                };
                parent.map(|parent| {
                    self.target_context(crate::style::PageStyleTarget {
                        element: parent.id(),
                        pseudo: None,
                    })
                })
            }
            Context::Root | Context::Element => None,
        }
    }

    fn context_tree(&self, context: Context) -> Tree<'a> {
        match context {
            Context::Tree(target) => Tree::Element(self.tree_element(target)),
            Context::Root | Context::Element => self.tree,
            Context::Page => Tree::Root,
        }
    }

    fn ensure_context(&self, context: Context) -> Result<(), StyleError> {
        if context == Context::Page {
            return self.ensure_page();
        }
        let mut pending = Vec::new();
        let mut ancestor = context;
        while let Context::Tree(target) = ancestor {
            if self.tree_contexts.borrow().contains_key(&target) {
                break;
            }
            pending.push(target);
            let Some(parent) = self.parent_context(ancestor) else {
                break;
            };
            ancestor = parent;
        }
        for target in pending.into_iter().rev() {
            let context = Context::Tree(target);
            let element = self.tree_element(target);
            if let Some(parent) = self.parent_context(context) {
                self.property(parent, Property::Display)?;
            }
            let parent = self.parent_style(context);
            let (style, values) = match target.pseudo {
                Some(pseudo) => {
                    self.sheet
                        .prepare_pseudo(element, &parent, pseudo, self.resolver)?
                }
                None => self
                    .sheet
                    .prepare_element(element, &parent, self.resolver)?,
            };
            self.tree_contexts
                .borrow_mut()
                .insert(target, Rc::new(Computed::new(style, values)?));
        }
        Ok(())
    }

    fn select_name(&self) -> Result<(), StyleError> {
        if self.selection_done.get() {
            return Ok(());
        }
        self.enter(Node::FirstPageStart)?;
        let selected = self.select_first.expect("a deferred page has a selector")(self);
        self.active.borrow_mut().pop();
        self.first_request.borrow_mut().name = selected?;
        self.selection_done.set(true);
        Ok(())
    }

    fn select_side(&self) -> Result<(), StyleError> {
        if self.side_selection_done.get() {
            return Ok(());
        }
        self.enter(Node::FirstPageSide)?;
        let selected = self
            .select_side
            .expect("a deferred page has a side selector")(self);
        self.active.borrow_mut().pop();
        self.first_request.borrow_mut().side = selected?;
        self.side_selection_done.set(true);
        Ok(())
    }

    fn ensure_page(&self) -> Result<(), StyleError> {
        if self.page.get().is_some() {
            return Ok(());
        }
        if !self.side_selection_done.get()
            && self.active.borrow().contains(&Node::FirstPageStart)
            && self.sheet.page_rules.iter().any(|rule| {
                rule.depends_on_side(&self.first_request.borrow(), self.selection_done.get())
            })
        {
            self.select_side()?;
        }
        let name_dependent = self.sheet.page_rules.iter().any(|rule| {
            rule.depends_on_name(&self.first_request.borrow(), self.side_selection_done.get())
        });
        if name_dependent {
            self.select_name()?;
        }
        if self.sheet.page_rules.iter().any(|rule| {
            rule.depends_on_side(&self.first_request.borrow(), self.selection_done.get())
        }) {
            self.select_side()?;
        }
        let rules = self.sheet.match_page(&self.first_request.borrow());
        let mut initial = self.root.base.anonymous(Display::Block);
        initial.text_decorations.propagated.clear();
        let page = Computed::new(initial, rules.declarations.clone())?;
        assert!(self.page_rules.set(rules).is_ok());
        assert!(self.page.set(Rc::new(page)).is_ok());
        Ok(())
    }

    fn counter_names(
        &self,
        context: Context,
        property: Property,
    ) -> Result<Vec<Arc<str>>, StyleError> {
        self.ensure_context(context)?;
        self.work.charge(1)?;
        let provided = (context == Context::Root && self.provided_root)
            || (context == Context::Element && self.provided_element);
        if !provided {
            let value = self.specified(context, property)?;
            if let Some(names) = self.project_counter_names(context, property, &value)? {
                return Ok(names);
            }
        }
        self.property(context, property)?;
        let computed = self.context(context);
        let style = computed.style.borrow();
        Ok(match property {
            Property::CounterReset => style
                .counters
                .reset
                .iter()
                .map(|change| change.name.clone())
                .collect(),
            Property::CounterIncrement => style
                .counters
                .increment
                .iter()
                .map(|change| change.name.clone())
                .collect(),
            Property::CounterSet => style
                .counters
                .set
                .iter()
                .map(|change| change.name.clone())
                .collect(),
            _ => unreachable!("counter name projection accepts only counter properties"),
        })
    }
    fn project_counter_names(
        &self,
        context: Context,
        property: Property,
        value: &Specified,
    ) -> Result<Option<Vec<Arc<str>>>, StyleError> {
        self.work.charge(1)?;
        match value {
            Specified::Value(value) => Ok(Some(crate::style::counter_values::names(value))),
            Specified::Wide(keyword) => Ok(Some(
                if keyword.resolve(property.inherited()) == Resolved::Inherit {
                    match self.parent_context(context) {
                        Some(parent) => self.counter_names(parent, property)?,
                        None => Vec::new(),
                    }
                } else {
                    Vec::new()
                },
            )),
            Specified::Toggle(values) => {
                let mut common = None;
                for value in values.iter() {
                    let Some(names) =
                        self.project_counter_names(context, property, &value.value)?
                    else {
                        return Ok(None);
                    };
                    if common.as_ref().is_some_and(|common| *common != names) {
                        return Ok(None);
                    }
                    common = Some(names);
                }
                Ok(common)
            }
            Specified::Unexpanded(_, _) => Ok(None),
            Specified::FlowRelative(..) => {
                unreachable!("counter properties belong to no logical property group")
            }
        }
    }

    fn selected_value<T>(
        &self,
        target: crate::style::PageStyleTarget,
        property: Property,
        value: impl FnOnce(&Style) -> T,
    ) -> Result<T, StyleError> {
        if self
            .document
            .and_then(|document| document.element(target.element))
            .is_none()
        {
            return Err(StyleError::Invalid(
                "first-page style target belongs to a different document".into(),
            ));
        }
        let context = self.target_context(target);
        self.property(context, property)?;
        Ok(value(&self.context(context).style.borrow()))
    }

    fn new(
        sheet: &'a Stylesheet,
        resolver: &'a dyn ResourceResolver,
        root: Computed,
        provided_root: bool,
        rules: Option<crate::style::page_rules::MatchedPageRules>,
        viewport: Option<[f32; 2]>,
    ) -> Result<Self, StyleError> {
        let page_rules = OnceCell::new();
        let page = OnceCell::new();
        if let Some(rules) = rules {
            let mut initial = root.base.anonymous(Display::Block);
            initial.text_decorations.propagated.clear();
            let computed = Computed::new(initial, rules.declarations.clone())?;
            assert!(page.set(Rc::new(computed)).is_ok());
            assert!(page_rules.set(rules).is_ok());
        }
        Ok(Self {
            sheet,
            page_rules,
            viewport,
            resolver,
            root: Rc::new(root),
            page,
            document: None,
            select_first: None,
            select_side: None,
            first_request: RefCell::new(sheet.first_page.clone()),
            selection_done: Cell::new(false),
            side_selection_done: Cell::new(false),
            tree_contexts: RefCell::default(),
            provided_root,
            provided_element: false,
            size: OnceCell::new(),
            size_source: RefCell::default(),
            element: None,
            parent: None,
            tree: Tree::Root,
            page_font: false,
            work: Rc::default(),
            complete: RefCell::default(),
            dimensions: RefCell::default(),
            active: RefCell::default(),
            axes: RefCell::default(),
        })
    }

    fn context(&self, context: Context) -> Rc<Computed> {
        match context {
            Context::Root => self.root.clone(),
            Context::Page => self
                .page
                .get()
                .expect("page selection precedes page values")
                .clone(),
            Context::Element => self
                .element
                .as_ref()
                .expect("an element context exists")
                .clone(),
            Context::Tree(target) => self
                .tree_contexts
                .borrow()
                .get(&target)
                .expect("tree context is prepared before evaluation")
                .clone(),
        }
    }

    fn source(&self, node: Node) -> Option<SourceLocation> {
        match node {
            Node::Property(context, property) => self
                .context(context)
                .sources
                .borrow()
                .get(&property)
                .cloned(),
            Node::Custom(context, index) => {
                let computed = self.context(context);
                computed
                    .customs
                    .current(&computed.customs.names[index])
                    .and_then(|winner| winner.location.as_deref().cloned())
            }
            Node::Paper(_) => self.size_source.borrow().clone().or_else(|| {
                self.page_rules
                    .get()
                    .and_then(|rules| rules.descriptor_source(Descriptor::PageSize))
            }),
            Node::Area(_) | Node::FirstPageStart | Node::FirstPageSide => None,
        }
    }

    fn enter(&self, node: Node) -> Result<(), StyleError> {
        let mut active = self.active.borrow_mut();
        if let Some(start) = active.iter().position(|candidate| *candidate == node) {
            let chain = active[start..].iter().copied().chain([node]);
            let mut sources = Vec::new();
            let mut dependencies = Vec::new();
            for dependency in chain {
                dependencies.push(dependency.name(self));
                if let Some(source) = self.source(dependency)
                    && !sources.contains(&source)
                {
                    sources.push(source.clone());
                }
            }
            return Err(StyleError::DependencyCycle {
                sources,
                dependencies,
            });
        }
        active.push(node);
        Ok(())
    }

    fn property(&self, mut context: Context, property: Property) -> Result<(), StyleError> {
        let active_start = self.active.borrow().len();
        let _work = self.work.enter(false);
        let result = (|| {
            let mut pending = Vec::new();
            loop {
                self.ensure_context(context)?;
                let node = Node::Property(context, property);
                if (context == Context::Root && self.provided_root)
                    || (context == Context::Element && self.provided_element)
                    || self.complete.borrow().contains(&node)
                {
                    break;
                }
                self.enter(node)?;
                pending.push(context);
                if property.inherited()
                    && let Some(parent) = self.parent_context(context)
                    && needs_parent(property, &self.specified(context, property)?)
                {
                    context = parent;
                    continue;
                }
                break;
            }
            for context in pending.into_iter().rev() {
                self.compute_property(context, property)?;
                self.active.borrow_mut().pop();
                self.complete
                    .borrow_mut()
                    .insert(Node::Property(context, property));
            }
            Ok(())
        })();
        self.active.borrow_mut().truncate(active_start);
        result
    }

    fn specified(&self, context: Context, property: Property) -> Result<Specified, StyleError> {
        self.ensure_context(context)?;
        let computed = self.context(context);
        if let Some(value) = computed.winners.borrow().get(&property) {
            return Ok(value.clone());
        }
        let winner = computed.properties.resolve_with(
            property,
            &mut || {
                if let Some(parent) = self.parent_context(context) {
                    self.property(parent, Property::TextAlignAll)?;
                }
                Ok(self.parent_style(context).align)
            },
            &mut || self.flows(context),
            &mut |pending, namespace| {
                computed
                    .sources
                    .borrow_mut()
                    .insert(property, pending.location.clone());
                let query = Query {
                    geometry: self,
                    context,
                    namespace,
                    namespaces: pending.namespaces.clone(),
                    base: Some(pending.location.base.clone()),
                };
                let Some(value) = self.substitute(context, &pending.value, &query)? else {
                    return Ok(None);
                };
                if computed.cyclic_properties.borrow().contains(&property) {
                    return Ok(None);
                }
                crate::style::declaration::expand_substitution(pending, &value)
            },
        )?;
        let value = if let Some(winner) = winner {
            if let Some(location) = winner.location {
                computed
                    .sources
                    .borrow_mut()
                    .insert(property, (*location).clone());
            }
            if property == Property::FontFamily
                && matches!(winner.value, Specified::Value(_) | Specified::Toggle(_))
            {
                computed.style.borrow_mut().font_namespace = winner.namespace;
            }
            if crate::style::anchor_values::namespaced(property, &winner.value) {
                computed
                    .style
                    .borrow_mut()
                    .anchor
                    .namespaces
                    .insert(property, winner.namespace);
            }
            winner.value
        } else {
            Specified::Wide(WideKeyword::Unset)
        };
        computed
            .winners
            .borrow_mut()
            .insert(property, value.clone());
        Ok(value)
    }

    /// The context's flow and its parent's, after their `direction` computes.
    fn flows(&self, context: Context) -> Result<Flows, StyleError> {
        self.property(context, Property::Direction)?;
        let element = Flow::of(&self.context(context).style.borrow());
        let parent = match self.parent_context(context) {
            Some(parent) => {
                self.property(parent, Property::Direction)?;
                Flow::of(&self.context(parent).style.borrow())
            }
            None => Flow::of(&self.parent_style(context)),
        };
        Ok(Flows { element, parent })
    }

    fn parent_style(&self, context: Context) -> Style {
        match context {
            Context::Root => Style::default(),
            Context::Tree(_) => self
                .parent_context(context)
                .map_or_else(Style::default, |parent| {
                    self.context(parent).style.borrow().clone()
                }),
            Context::Page => self.root.style.borrow().clone(),
            Context::Element => self
                .parent
                .expect("element inheritance has a parent style")
                .clone(),
        }
    }

    fn prerequisites(
        &self,
        context: Context,
        property: Property,
        value: &Specified,
    ) -> Result<(), StyleError> {
        if property == Property::Display {
            self.property(context, Property::WritingMode)?;
            if let Some(parent) = self.parent_context(context) {
                self.property(parent, Property::WritingMode)?;
                self.property(parent, Property::RubyPosition)?;
                let mode = self.context(parent).style.borrow().writing_mode;
                self.context(context).style.borrow_mut().parent_writing_mode = mode;
            }
            self.property(context, Property::Position)?;
            self.property(context, Property::Float)?;
        }
        // CSS UI 4 §3.2, §3.4: the outline's width and `auto` colour depend on its style.
        if matches!(property, Property::OutlineWidth | Property::OutlineColor) {
            self.property(context, Property::OutlineStyle)?;
        }
        if property == Property::OutlineColor {
            self.property(context, Property::Color)?;
        }
        // `x-start` and `x-end` name the element's inline-axis edges.
        if property == Property::BackgroundPositionX {
            self.property(context, Property::Direction)?;
        }
        if let Some(parent) = self.parent_context(context)
            && needs_parent(property, value)
        {
            if matches_parent_alignment(property, value) {
                self.property(parent, Property::Direction)?;
            }
            let source = match value {
                Specified::FlowRelative(flow, _) => logical::corresponding(
                    property,
                    *flow,
                    Flow::of(&self.context(parent).style.borrow()),
                ),
                _ => property,
            };
            self.property(parent, source)?;
        }
        if current_color(property, value) {
            if property != Property::Color {
                self.property(context, Property::Color)?;
            } else if let Some(parent) = self.parent_context(context) {
                self.property(parent, Property::Color)?;
            }
        }
        Ok(())
    }

    fn apply(
        &self,
        context: Context,
        property: Property,
        value: &Specified,
        namespace: Option<crate::fonts::FontNamespace>,
    ) -> Result<Style, StyleError> {
        self.prerequisites(context, property, value)?;
        let mut target = self.context(context).style.borrow().clone();
        if property == Property::FontFamily
            && let Some(namespace) = namespace
        {
            target.font_namespace = namespace;
        }
        let parent = self.parent_style(context);
        self.with_relative(context, property, |relative| {
            properties::apply(property, value, &mut target, &parent, relative)
        })?;
        Ok(target)
    }

    fn compute_property(&self, context: Context, property: Property) -> Result<(), StyleError> {
        if let Some(side) = Property::BorderWidth
            .longhands()
            .iter()
            .position(|candidate| *candidate == property)
        {
            self.property(context, Property::BorderStyle.longhands()[side])?;
            if self.context(context).style.borrow().decoration.borders[side]
                .style
                .suppresses_width()
            {
                return Ok(());
            }
        }
        let value = self.specified(context, property)?;
        if matches!(property, Property::OverflowX | Property::OverflowY) {
            let axis = usize::from(property == Property::OverflowY);
            let style = self.context(context).style.borrow().clone();
            let parent = self.parent_style(context);
            let value = self.with_relative(context, property, |relative| {
                crate::style::overflow::compute_axis(axis, &style, &parent, relative, |axis| {
                    self.specified(context, [Property::OverflowX, Property::OverflowY][axis])
                        .map(crate::style::overflow::AxisValue::Specified)
                })
            })?;
            self.context(context).style.borrow_mut().overflow[axis] = value;
            return Ok(());
        }
        let target = self.apply(context, property, &value, None)?;
        properties::copy(
            property,
            &mut self.context(context).style.borrow_mut(),
            &target,
        );
        Ok(())
    }

    fn with_relative<T>(
        &self,
        context: Context,
        property: Property,
        compute: impl FnOnce(&Relative<'_>) -> Result<T, StyleError>,
    ) -> Result<T, StyleError> {
        let font_property = property == Property::FontSize || FONT_SELECTION.contains(&property);
        let own_font = FontContext {
            geometry: self,
            context,
            line: if property == Property::LineHeight {
                self.parent_context(context)
            } else {
                Some(context)
            },
            parent_line: context == Context::Element && property == Property::LineHeight,
        };
        let root_font = FontContext {
            geometry: self,
            context: Context::Root,
            line: if context == Context::Root && property == Property::LineHeight {
                None
            } else {
                Some(Context::Root)
            },
            parent_line: false,
        };
        let color_scheme = self
            .sheet
            .used_color_scheme(&self.context(context).style.borrow().color_scheme);
        let parent_context = self.parent_context(context).unwrap_or(Context::Root);
        let parent_font = FontContext {
            geometry: self,
            context: parent_context,
            line: Some(parent_context),
            parent_line: false,
        };
        if context == Context::Element {
            let parent = self.parent.expect("element inheritance has a parent style");
            let viewport = self.viewport.unwrap_or_else(|| self.sheet.page_area());
            let inherited = self.sheet.inherited(parent, viewport, self.resolver);
            let root = if parent.root_font.is_none() && !font_property {
                FontBasis::Deferred(&own_font)
            } else {
                inherited.root
            };
            return compute(&Relative {
                physical_unit_scale: inherited.physical_unit_scale,
                font: if font_property {
                    inherited.font
                } else {
                    FontBasis::Deferred(&own_font)
                },
                root,
                viewport: Viewport::Deferred(self),
                paper: self.sheet.paper,
                tree: self.tree,
                grammar_only: false,
                work: self.work.clone(),
                initial_page_font_size: self.page_font && property == Property::FontSize,
                initial_font: self.sheet.environment.relative.initial_font.clone(),
                color_scheme,
                counter_styles: Some(self.sheet.counter_styles.clone()),
                accent_color: inherited.accent_color,
                containers: inherited.containers.clone(),
                query_sizes: inherited.query_sizes,
                anchoring: inherited.anchoring,
            });
        }
        compute(&Relative {
            physical_unit_scale: self.sheet.environment.relative.physical_unit_scale,
            font: if font_property {
                if context == Context::Root {
                    self.sheet.initial_relative().font
                } else {
                    FontBasis::Deferred(&parent_font)
                }
            } else {
                FontBasis::Deferred(&own_font)
            },
            root: if context == Context::Root && font_property {
                self.sheet.initial_relative().font
            } else {
                FontBasis::Deferred(&root_font)
            },
            viewport: Viewport::Deferred(self),
            paper: self.sheet.paper,
            tree: self.context_tree(context),
            grammar_only: false,
            work: self.work.clone(),
            initial_page_font_size: context == Context::Page && property == Property::FontSize,
            initial_font: self.sheet.environment.relative.initial_font.clone(),
            color_scheme,
            counter_styles: Some(self.sheet.counter_styles.clone()),
            accent_color: None,
            containers: Default::default(),
            query_sizes: Some(&self.sheet.query_sizes),
            anchoring: None,
        })
    }

    fn custom(
        &self,
        mut context: Context,
        name: &str,
    ) -> Result<Option<variables::Expanded>, StyleError> {
        let _work = self.work.enter(false);
        loop {
            self.ensure_context(context)?;
            if (context == Context::Root && self.provided_root)
                || (context == Context::Element && self.provided_element)
            {
                return Ok(self
                    .context(context)
                    .base
                    .custom_properties
                    .get(name)
                    .and_then(Clone::clone));
            }
            let computed = self.context(context);
            if let Some(dependencies) = computed.font_pending.borrow().get(name).copied()
                && let Some(property) =
                    self.active
                        .borrow()
                        .iter()
                        .rev()
                        .find_map(|node| match *node {
                            Node::Property(active, property)
                                if active == context
                                    && ((property == Property::FontSize
                                        && dependencies.font_size)
                                        || (property == Property::LineHeight
                                            && dependencies.line_height)) =>
                            {
                                Some(property)
                            }
                            _ => None,
                        })
            {
                computed.cyclic_properties.borrow_mut().insert(property);
                computed.cyclic_customs.borrow_mut().insert(name.to_owned());
                return Ok(None);
            }
            self.work.charge(1)?;
            if let Some((name, keyword)) = computed.variables.compute(
                name,
                &CustomEvaluation {
                    geometry: self,
                    context,
                },
            )? {
                return Err(StyleError::Invalid(format!(
                    "unresolved custom property keyword {name}: {}",
                    keyword.name()
                )));
            }
            if let Some(value) = computed.variables.local(name) {
                return Ok(value);
            }
            match self.parent_context(context) {
                Some(parent) => context = parent,
                None => return self.parent_custom(context, name),
            }
        }
    }

    /// The computed value of the registered custom property `name` from its substituted
    /// `value` (CSS Properties and Values API 1 §2.4); `None` when invalid at computed-value
    /// time, which includes closing a font-relative dependency cycle (§2.7.2).
    fn registered(
        &self,
        context: Context,
        name: &str,
        registration: &Registration,
        value: &variables::Expanded,
        base: Option<&crate::resources::ResourceBase>,
    ) -> Result<Option<variables::Expanded>, StyleError> {
        let computed = self.context(context);
        let dependencies =
            crate::style::registration::font_dependencies(value.as_str(), self.is_root(context));
        for (dependent, property) in [
            (dependencies.font_size, Property::FontSize),
            (dependencies.line_height, Property::LineHeight),
        ] {
            if dependent
                && self
                    .active
                    .borrow()
                    .contains(&Node::Property(context, property))
            {
                computed.cyclic_properties.borrow_mut().insert(property);
                return Ok(None);
            }
        }
        computed
            .font_pending
            .borrow_mut()
            .insert(name.to_owned(), dependencies);
        // Lengths resolve against the element's own font, as in a non-font property.
        let result = self.with_relative(context, Property::Width, |relative| {
            crate::style::registration::compute(&registration.syntax, value, relative, base, name)
        });
        computed.font_pending.borrow_mut().remove(name);
        if computed.cyclic_customs.borrow().contains(name) {
            return Ok(None);
        }
        result
    }

    /// Whether `context` is the root element, whose root-relative units name its own font.
    fn is_root(&self, context: Context) -> bool {
        match context {
            Context::Root => true,
            Context::Element => self.parent.is_some_and(|parent| parent.root_font.is_none()),
            Context::Page | Context::Tree(_) => false,
        }
    }

    fn parent_custom(
        &self,
        context: Context,
        name: &str,
    ) -> Result<Option<variables::Expanded>, StyleError> {
        match context {
            Context::Page => self.custom(Context::Root, name),
            Context::Tree(_) => match self.parent_context(context) {
                Some(parent) => self.custom(parent, name),
                None => Ok(None),
            },
            // The element's inherited environment is its parent's, carrying the document's
            // registrations even for the root.
            Context::Element => Ok(self
                .context(Context::Element)
                .base
                .inherited_custom_properties
                .get(name)
                .and_then(Clone::clone)),
            Context::Root => Ok(self
                .root
                .base
                .inherited_custom_properties
                .get(name)
                .and_then(Clone::clone)),
        }
    }

    fn substitute(
        &self,
        context: Context,
        value: &variables::Value,
        query: &Query<'_, '_>,
    ) -> Result<Option<variables::Expanded>, StyleError> {
        self.work.check_value_nesting()?;
        let _work = self.work.enter(false);
        self.work.charge(1)?;
        value.substitute_context(
            &self.sheet.registrations,
            &mut |reference, name| match reference {
                Reference::Local => self.custom(context, name),
                Reference::Inherited => self.parent_custom(context, name),
            },
            &mut |test| {
                self.work.charge(1)?;
                test.evaluate(&self.sheet.environment, query.namespaces.as_ref(), query)
            },
            &mut |attribute| attribute.resolve(query),
        )
    }

    fn custom_environment(&self, context: Context) -> Result<variables::Environment, StyleError> {
        if context == Context::Root && self.provided_root {
            return Ok(self.root.base.custom_properties.clone());
        }
        let computed = self.context(context);
        for name in computed.variables.names() {
            self.custom(context, &name)?;
        }
        let parent = match context {
            Context::Page => self.custom_environment(Context::Root)?,
            Context::Tree(_) => match self.parent_context(context) {
                Some(parent) => self.custom_environment(parent)?,
                None => Default::default(),
            },
            Context::Element | Context::Root => computed.base.inherited_custom_properties.clone(),
        };
        let environment = computed.variables.environment(&parent)?;
        computed.style.borrow_mut().custom_properties = environment.clone();
        Ok(environment)
    }

    fn materialized(&self, context: Context) -> Result<(Style, Winners), StyleError> {
        let computed = self.context(context);
        let environment = self.custom_environment(context)?;
        for property in computed.properties.properties() {
            self.specified(context, property)?;
        }
        let mut base = computed.base.clone();
        base.custom_properties = environment;
        base.inherited_custom_properties = match context {
            Context::Page => self.custom_environment(Context::Root)?,
            Context::Tree(_) => match self.parent_context(context) {
                Some(parent) => self.custom_environment(parent)?,
                None => Default::default(),
            },
            Context::Element | Context::Root => base.inherited_custom_properties,
        };
        base.font_namespace = computed.style.borrow().font_namespace;
        base.anchor
            .namespaces
            .clone_from(&computed.style.borrow().anchor.namespaces);
        Ok((base, computed.winners.borrow().clone()))
    }

    fn page_size(&self) -> Result<Option<Arc<Component>>, StyleError> {
        if let Some(value) = self.size.get() {
            return Ok(value.clone());
        }
        let (value, source) = self
            .page_rules
            .get()
            .expect("page geometry owns matched page rules")
            .descriptor_with(
                Descriptor::PageSize,
                |value, namespace, namespaces, location| {
                    *self.size_source.borrow_mut() = location.cloned();
                    self.substitute(
                        Context::Page,
                        value,
                        &Query {
                            geometry: self,
                            context: Context::Page,
                            namespace,
                            namespaces: namespaces.cloned(),
                            base: location.map(|location| location.base.clone()),
                        },
                    )
                },
            )?;
        *self.size_source.borrow_mut() = source;
        Ok(self.size.get_or_init(|| value).clone())
    }

    fn font_unit(&self, context: Context, unit: FontUnit) -> Result<f32, StyleError> {
        if unit == FontUnit::Lh {
            self.property(context, Property::LineHeight)?;
            let height = self.context(context).style.borrow().line_height;
            if let LineHeight::Length(length) = height {
                return Ok(length);
            }
            self.property(context, Property::FontSize)?;
            let size = self.context(context).style.borrow().size;
            if let Some(height) = height.resolve(size) {
                return Ok(height);
            }
            let basis = self.font_basis(context, size, height, true)?;
            return basis.per_unit(unit);
        }
        self.property(context, Property::FontSize)?;
        let size = self.context(context).style.borrow().size;
        if unit == FontUnit::Em {
            return Ok(size);
        }
        self.property(context, Property::WritingMode)?;
        self.property(context, Property::TextOrientation)?;
        let vertical = self.context(context).style.borrow().vertical_font_units();
        self.font_basis(context, size, LineHeight::Normal, true)?
            .with_vertical_units(vertical)
            .per_unit(unit)
    }

    fn size_metric(&self, context: Context, metric: SizeMetric) -> Result<f32, StyleError> {
        self.property(context, Property::FontSize)?;
        let size = self.context(context).style.borrow().size;
        self.font_basis(context, size, LineHeight::Normal, false)?
            .size_metric(metric)
    }

    /// The context's font; without `font-size-adjust` where `size_adjusted` is false, as
    /// `from-font` measures it.
    fn font_basis(
        &self,
        context: Context,
        size: f32,
        height: LineHeight,
        size_adjusted: bool,
    ) -> Result<FontBasis<'a>, StyleError> {
        for property in FONT_SELECTION.iter().chain(&FONT_ADJUSTMENT) {
            if size_adjusted || *property != Property::FontSizeAdjust {
                self.property(context, *property)?;
            }
        }
        let computed = self.context(context);
        let style = computed.style.borrow();
        let selection = style.font_selection();
        let candidates = self.sheet.fonts.candidates_in_namespace(
            style.font_namespace,
            &style.families,
            selection,
        );
        let mut adjustments = style.font_adjustments.clone();
        if !size_adjusted {
            adjustments.size_adjust = None;
        }
        Ok(self.sheet.basis(
            size,
            height,
            &candidates,
            selection,
            &adjustments,
            self.resolver,
        ))
    }

    fn dimension(&self, node: Node) -> Result<f32, StyleError> {
        self.ensure_page()?;
        if let Some(value) = self.dimensions.borrow().get(&node) {
            return Ok(*value);
        }
        self.enter(node)?;
        let result = self.compute_dimension(node);
        self.active.borrow_mut().pop();
        if let Ok(value) = result {
            self.dimensions.borrow_mut().insert(node, value);
        }
        result
    }

    fn page_axis(&self, axis: Axis) -> Result<f32, StyleError> {
        let basis = self.dimension(Node::Paper(axis))?;
        let index = usize::from(axis != Axis::Width);
        let sides = if axis == Axis::Width { [3, 1] } else { [0, 2] };
        for side in sides {
            for property in [
                PAGE_MARGINS[side],
                PADDING[side],
                Property::BorderWidth.longhands()[side],
                Property::BorderStyle.longhands()[side],
            ] {
                self.property(Context::Page, property)?;
            }
        }
        let properties = if axis == Axis::Width {
            [Property::Width, Property::MinWidth, Property::MaxWidth]
        } else {
            [Property::Height, Property::MinHeight, Property::MaxHeight]
        };
        for property in properties {
            self.property(Context::Page, property)?;
        }
        self.property(Context::Page, Property::BoxSizing)?;
        let computed = self.context(Context::Page);
        let style = computed.style.borrow();
        let margins = sides.map(|side| style.margin[side].resolve(basis));
        let edges = sides
            .iter()
            .map(|&side| {
                style.padding[side].resolve(basis) + style.decoration.borders[side].used_width()
            })
            .sum();
        let sizes = if axis == Axis::Width {
            [&style.width, &style.min_width, &style.max_width]
        } else {
            [&style.height, &style.min_height, &style.max_height]
        };
        if let Some(property) = properties
            .iter()
            .zip(sizes)
            .find_map(|(property, size)| matches!(size, Size::Intrinsic(_)).then_some(property))
        {
            return Err(StyleError::Unsupported(format!(
                "@page {}: intrinsic size keyword",
                property.name()
            )));
        }
        let constraints =
            crate::style::SizeConstraints::new(sizes, Some(basis), style.box_sizing, edges);
        let used = constraints.normal_flow(
            basis,
            edges,
            margins,
            None,
            false,
            crate::style::Overconstraint::ResizeContaining,
        );
        self.axes.borrow_mut()[index] = Some(used);
        Ok(used.size)
    }

    fn compute_dimension(&self, node: Node) -> Result<f32, StyleError> {
        match node {
            Node::Paper(axis) => match self.page_size()? {
                None => Ok(self.sheet.paper[usize::from(axis != Axis::Width)]),
                Some(value) => {
                    let page_font = FontContext {
                        geometry: self,
                        context: Context::Page,
                        line: Some(Context::Page),
                        parent_line: false,
                    };
                    let root_font = FontContext {
                        geometry: self,
                        context: Context::Root,
                        line: Some(Context::Root),
                        parent_line: false,
                    };
                    let relative = Relative {
                        physical_unit_scale: self.sheet.environment.relative.physical_unit_scale,
                        font: FontBasis::Deferred(&page_font),
                        root: FontBasis::Deferred(&root_font),
                        viewport: Viewport::Deferred(self),
                        paper: self.sheet.paper,
                        tree: Tree::Root,
                        grammar_only: false,
                        work: Default::default(),
                        initial_page_font_size: false,
                        initial_font: self.sheet.environment.relative.initial_font.clone(),
                        color_scheme: self.sheet.used_color_scheme(
                            &self.context(Context::Page).style.borrow().color_scheme,
                        ),
                        counter_styles: Some(self.sheet.counter_styles.clone()),
                        accent_color: None,
                        containers: Default::default(),
                        query_sizes: None,
                        anchoring: None,
                    };
                    crate::style::page_values::page_dimension(
                        &value,
                        axis,
                        &relative,
                        self.sheet.paper,
                    )
                }
            },
            Node::Area(axis) => self.page_axis(axis),
            Node::Custom(..) | Node::Property(..) | Node::FirstPageStart | Node::FirstPageSide => {
                unreachable!("property nodes have computed styles")
            }
        }
    }

    fn finish(&self) -> Result<PageGeometry, StyleError> {
        let area = [
            self.dimension(Node::Area(Axis::Width))?,
            self.dimension(Node::Area(Axis::Height))?,
        ];
        let axes = self.axes.borrow();
        let [width, height] =
            axes.map(|axis| axis.expect("resolved page area has an axis solution"));
        let extent = [width.containing, height.containing];
        let margin = [
            height.margins[0],
            width.margins[1],
            height.margins[1],
            width.margins[0],
        ];
        let specified = [
            self.dimension(Node::Paper(Axis::Width))?,
            self.dimension(Node::Paper(Axis::Height))?,
        ];
        let paper = std::array::from_fn(|axis| {
            let dimension = specified[axis].max(extent[axis]);
            if dimension == 0.0 {
                self.sheet.paper[axis]
            } else {
                dimension
            }
        });
        let computed = self.context(Context::Page);
        let style = computed.style.borrow();
        let padding = std::array::from_fn(|side| {
            style.padding[side].resolve(specified[usize::from(side.is_multiple_of(2))])
        });
        let border = style
            .decoration
            .borders
            .map(crate::paint::Border::used_width);
        if paper
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
            || area.iter().any(|value| !value.is_finite() || *value < 0.0)
            || extent.iter().any(|value| !value.is_finite())
            || margin
                .iter()
                .chain(padding.iter())
                .chain(border.iter())
                .any(|value| !value.is_finite())
        {
            return Err("invalid page content area".into());
        }
        Ok(PageGeometry {
            paper,
            extent,
            area,
            margin,
            padding,
            border,
        })
    }
}

impl crate::style::PageSelection for Geometry<'_> {
    fn display(&self, target: crate::style::PageStyleTarget) -> Result<Display, StyleError> {
        self.selected_value(target, Property::Display, |style| style.display.clone())
    }
    fn position(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<crate::style::Position, StyleError> {
        self.selected_value(target, Property::Position, |style| style.position)
    }
    fn page(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<crate::style::Page, StyleError> {
        self.selected_value(target, Property::Page, |style| style.page.clone())
    }
    fn break_before(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<crate::style::BreakBetween, StyleError> {
        self.selected_value(target, Property::BreakBefore, |style| style.break_before)
    }
    fn progression_rtl(&self) -> Result<bool, StyleError> {
        let document = self.document.expect("page selection has a document");
        let element = document
            .body_element()
            .unwrap_or_else(|| document.root_element());
        let target = crate::style::PageStyleTarget {
            element: element.id(),
            pseudo: None,
        };
        let rtl = self.selected_value(target, Property::Direction, |style| style.rtl)?;
        let mode =
            self.selected_value(target, Property::WritingMode, |style| style.writing_mode)?;
        Ok(mode.page_progression_rtl(rtl))
    }
    fn order(&self, target: crate::style::PageStyleTarget) -> Result<i32, StyleError> {
        self.selected_value(target, Property::Order, |style| style.flex.order)
    }
    fn caption_side(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<crate::style::CaptionSide, StyleError> {
        self.selected_value(target, Property::CaptionSide, |style| {
            style.table.caption_side
        })
    }
    fn white_space_collapse(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<crate::style::WhiteSpaceCollapse, StyleError> {
        self.selected_value(target, Property::WhiteSpaceCollapse, |style| {
            style.white_space.collapse
        })
    }
    fn content(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<Option<crate::style::GeneratedContent>, StyleError> {
        self.selected_value(target, Property::Content, |style| {
            match style.computed_content.as_ref() {
                crate::style::generated::ComputedContent::Items { content, .. } => {
                    Some(content.clone())
                }
                _ => None,
            }
        })
    }
    fn bookmark_label(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<Option<crate::style::GeneratedContent>, StyleError> {
        if self
            .selected_value(target, Property::BookmarkLevel, |style| {
                style.bookmark.level
            })?
            .is_none()
        {
            return Ok(None);
        }
        self.selected_value(target, Property::BookmarkLabel, |style| {
            Some(style.bookmark.label.clone())
        })
    }
    fn string_set(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<Arc<[crate::style::StringAssignment]>, StyleError> {
        self.selected_value(target, Property::StringSet, |style| {
            style.string_set.clone()
        })
    }
    fn counter_scopes(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<crate::style::CounterScopes, StyleError> {
        let context = self.target_context(target);
        let reset = self.counter_names(context, Property::CounterReset)?;
        let mut updates = self.counter_names(context, Property::CounterIncrement)?;
        updates.extend(self.counter_names(context, Property::CounterSet)?);
        Ok(crate::style::CounterScopes { reset, updates })
    }
    fn counters(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<crate::style::CounterProperties, StyleError> {
        Ok(crate::style::CounterProperties {
            reset: self.selected_value(target, Property::CounterReset, |style| {
                style.counters.reset.clone()
            })?,
            increment: self.selected_value(target, Property::CounterIncrement, |style| {
                style.counters.increment.clone()
            })?,
            set: self.selected_value(target, Property::CounterSet, |style| {
                style.counters.set.clone()
            })?,
        })
    }
    fn quotes(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<crate::style::Quotes, StyleError> {
        self.selected_value(target, Property::Quotes, |style| style.quotes.clone())
    }
    fn style_containment(&self, target: crate::style::PageStyleTarget) -> Result<bool, StyleError> {
        let contain =
            self.selected_value(target, Property::Contain, |style| style.containment().style)?;
        let container = self.selected_value(target, Property::ContainerType, |style| {
            style.containment().style
        })?;
        let visibility = self.selected_value(target, Property::ContentVisibility, |style| {
            style.containment().style
        })?;
        Ok(contain || container || visibility)
    }
    fn inline_edges(
        &self,
        target: crate::style::PageStyleTarget,
        continued: [bool; 2],
    ) -> Result<bool, StyleError> {
        self.selected_value(target, Property::Direction, |_| ())?;
        for property in Property::Padding
            .longhands()
            .iter()
            .chain(Property::BorderStyle.longhands())
            .chain(Property::BorderWidth.longhands())
            .chain(Property::Margin.longhands())
        {
            if self.selected_value(target, *property, |style| style.has_inline_edges(continued))? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn tab_size(
        &self,
        target: crate::style::PageStyleTarget,
    ) -> Result<crate::style::TabSize, StyleError> {
        self.selected_value(target, Property::TabSize, |style| style.tab_size)
    }
    fn inside_marker(&self, target: crate::style::PageStyleTarget) -> Result<bool, StyleError> {
        if self.selected_value(target, Property::ListStylePosition, |style| {
            style.list_style_position
        })? != crate::style::ListStylePosition::Inside
        {
            return Ok(false);
        }
        let marker = crate::style::PageStyleTarget {
            pseudo: Some(crate::style::PseudoElement::Marker),
            ..target
        };
        match self
            .selected_value(marker, Property::Content, |style| {
                style.computed_content.clone()
            })?
            .as_ref()
        {
            crate::style::generated::ComputedContent::None => Ok(false),
            crate::style::generated::ComputedContent::Items { .. } => Ok(true),
            crate::style::generated::ComputedContent::Contents => {
                Ok(
                    !self.selected_value(target, Property::ListStyleType, |style| {
                        style.list_style_type.is_none()
                    })? || self.selected_value(target, Property::ListStyleImage, |style| {
                        style.list_style_image.is_displayable()
                    })?,
                )
            }
        }
    }
}

impl ViewportUnits for Geometry<'_> {
    fn logical_dimension(&self, block: bool) -> Result<f32, StyleError> {
        self.property(Context::Root, Property::WritingMode)?;
        let vertical = self.root.style.borrow().writing_mode.vertical();
        <Self as ViewportUnits>::dimension(
            self,
            if block == vertical {
                Axis::Width
            } else {
                Axis::Height
            },
        )
    }
    fn dimension(&self, axis: Axis) -> Result<f32, StyleError> {
        if let Some([width, height]) = self.viewport {
            return Ok(match axis {
                Axis::Width => width,
                Axis::Height => height,
                Axis::Min => width.min(height),
                Axis::Max => width.max(height),
            });
        }
        match axis {
            Axis::Width | Axis::Height => self.dimension(Node::Area(axis)),
            Axis::Min => Ok(self
                .dimension(Node::Area(Axis::Width))?
                .min(self.dimension(Node::Area(Axis::Height))?)),
            Axis::Max => Ok(self
                .dimension(Node::Area(Axis::Width))?
                .max(self.dimension(Node::Area(Axis::Height))?)),
        }
    }
}

struct FontContext<'g, 'a> {
    geometry: &'g Geometry<'a>,
    context: Context,
    line: Option<Context>,
    parent_line: bool,
}
impl FontUnits for FontContext<'_, '_> {
    fn per_unit(&self, unit: FontUnit) -> Result<f32, StyleError> {
        if unit == FontUnit::Lh {
            if self.parent_line {
                return self
                    .geometry
                    .sheet
                    .inherited(
                        self.geometry
                            .parent
                            .expect("element line height has a parent"),
                        self.geometry.sheet.page_area(),
                        self.geometry.resolver,
                    )
                    .font
                    .per_unit(unit);
            }
            match self.line {
                Some(context) => self.geometry.font_unit(context, unit),
                None => self.geometry.sheet.initial_relative().font.per_unit(unit),
            }
        } else {
            self.geometry.font_unit(self.context, unit)
        }
    }
    fn size_metric(&self, metric: SizeMetric) -> Result<f32, StyleError> {
        self.geometry.size_metric(self.context, metric)
    }
}

#[cfg(test)]
pub(super) fn provided(
    sheet: &Stylesheet,
    root: &Style,
    resolver: &dyn ResourceResolver,
) -> Result<PageGeometry, StyleError> {
    let root = Computed::provided(root.clone())?;
    Geometry::new(
        sheet,
        resolver,
        root,
        true,
        Some(sheet.match_page(&sheet.first_page)),
        None,
    )?
    .finish()
}

pub(super) fn page(
    sheet: &Stylesheet,
    root: &Style,
    rules: crate::style::page_rules::MatchedPageRules,
    viewport: Option<[f32; 2]>,
    resolver: &dyn ResourceResolver,
) -> Result<(PageGeometry, Style), StyleError> {
    let mut root = root.clone();
    if root.root_font.is_none() {
        sheet.capture_root_font(&mut root);
    }
    let contextual = rules.contextual_descriptors;
    let graph = Geometry::new(
        sheet,
        resolver,
        Computed::provided(root.clone())?,
        true,
        Some(rules),
        viewport,
    )?;
    let geometry = graph.finish()?;
    let (mut page, mut winners) = graph.materialized(Context::Page)?;
    winners.remove(&Property::Display);
    let mut inherited = sheet.inherited(&root, viewport.unwrap_or(geometry.area), resolver);
    inherited.initial_page_font_size = true;
    let relative = sheet.text_basis(&mut page, &root, &mut winners, inherited, resolver)?;
    let mut page = Stylesheet::apply_properties(page, &root, winners, &relative)?;
    if contextual {
        page.page_parent = Some(Arc::new(root));
    }
    graph
        .page_rules
        .get()
        .expect("page geometry owns matched page rules")
        .validate_descriptors(sheet, &page, viewport.unwrap_or(geometry.area), resolver)?;
    Ok((geometry, page))
}

impl Stylesheet {
    /// Evaluates document values against already resolved page geometry.
    pub(crate) fn with_document_values<T>(
        &self,
        document: &DocumentTree,
        root: &Style,
        resolver: &dyn ResourceResolver,
        evaluate: impl FnOnce(&dyn crate::style::PageSelection) -> Result<T, StyleError>,
    ) -> Result<T, StyleError> {
        let mut graph = Geometry::new(
            self,
            resolver,
            Computed::provided(root.clone())?,
            true,
            None,
            Some(self.page_area()),
        )?;
        graph.document = Some(document);
        graph.tree = Tree::Element(document.root_element());
        graph.selection_done.set(true);
        graph.side_selection_done.set(true);
        evaluate(&graph)
    }

    /// Selects the first generated page in the same graph as its viewport dependencies.
    pub(crate) fn resolve_document_root_with_pages(
        &mut self,
        document: &DocumentTree,
        resolver: &dyn ResourceResolver,
        select: &crate::style::page_selection::SelectFirstPageName<'_>,
        side: &crate::style::page_selection::SelectFirstPageSide<'_>,
    ) -> Result<Style, StyleError> {
        let element = document.root_element();
        let (style, values) = self.prepare_element(element, &Style::default(), resolver)?;
        let mut graph = Geometry::new(
            self,
            resolver,
            Computed::new(style, values)?,
            false,
            None,
            None,
        )?;
        graph.document = Some(document);
        graph.select_first = Some(select);
        graph.select_side = Some(side);
        graph.tree = Tree::Element(element);
        graph.select_name()?;
        graph.select_side()?;
        let geometry = graph.finish()?;
        let (style, winners) = graph.materialized(Context::Root)?;
        let request = graph.first_request.borrow().clone();
        let progression_rtl = crate::style::PageSelection::progression_rtl(&graph)?;
        drop(graph);
        self.first_page = request;
        self.page_progression_rtl = progression_rtl;
        self.set_page_geometry(&geometry);
        let root = self.computed_values(element, &Style::default(), style, winners, resolver)?;
        self.define_palettes(&root, resolver)?;
        Ok(root)
    }

    pub(super) fn set_page_geometry(&mut self, geometry: &PageGeometry) {
        self.page = geometry.paper;
        self.page_extent = geometry.extent;
        self.page_content_size = geometry.area;
        self.margin = geometry.margin;
        self.page_padding = geometry.padding;
        self.page_border = geometry.border;
    }

    /// Resolves the first page before computing root properties that cannot affect its geometry.
    #[cfg(test)]
    pub(crate) fn resolve_document_root(
        &mut self,
        element: Element<'_>,
        resolver: &dyn ResourceResolver,
    ) -> Result<Style, StyleError> {
        let (style, values) = self.prepare_element(element, &Style::default(), resolver)?;
        let root = Computed::new(style, values)?;
        let mut graph = Geometry::new(
            self,
            resolver,
            root,
            false,
            Some(self.match_page(&self.first_page)),
            None,
        )?;
        graph.tree = Tree::Element(element);
        let geometry = graph.finish()?;
        let (style, winners) = graph.materialized(Context::Root)?;
        drop(graph);
        self.set_page_geometry(&geometry);
        let root = self.computed_values(element, &Style::default(), style, winners, resolver)?;
        self.define_palettes(&root, resolver)?;
        Ok(root)
    }
}

struct Query<'g, 'a> {
    geometry: &'g Geometry<'a>,
    context: Context,
    namespace: crate::fonts::FontNamespace,
    namespaces: Option<crate::style::selector::Namespaces>,
    /// The declaring stylesheet's location, which relative URLs in queried values take.
    base: Option<crate::resources::ResourceBase>,
}
impl crate::style::attribute::Context for Query<'_, '_> {
    fn attribute_data(&self, prefix: Option<&str>, name: &str) -> Option<&str> {
        if self.context == Context::Page {
            return None;
        }
        let Tree::Element(element) = self.geometry.context_tree(self.context) else {
            return None;
        };
        if let Some(prefix) = prefix {
            let namespaces = self.namespaces.as_ref()?.borrow();
            element.attr_in_namespace(Some(namespaces.get(prefix)?), name)
        } else {
            element.attr_in_namespace(None, name)
        }
    }
    fn expand_attribute_value(
        &self,
        value: &variables::Value,
    ) -> Result<Option<variables::Expanded>, StyleError> {
        self.geometry.substitute(self.context, value, self)
    }
    fn matches_attribute_syntax(
        &self,
        syntax: &crate::style::syntax::Syntax,
        value: &variables::Expanded,
    ) -> Result<bool, StyleError> {
        self.geometry
            .with_relative(self.context, Property::Width, |relative| {
                let mut steps = 0;
                let result = crate::style::matcher::match_syntax(
                    syntax,
                    value.as_str(),
                    &mut steps,
                    crate::style::matcher::ValueContext::Nested(relative),
                );
                relative.work.charge(steps)?;
                result
            })
    }
}
impl QueryValues for Query<'_, '_> {
    fn property(&self, property: Property) -> Result<Style, StyleError> {
        self.geometry.property(self.context, property)?;
        Ok(self.geometry.context(self.context).style.borrow().clone())
    }
    fn candidate(&self, property: Property, value: &Specified) -> Result<Style, StyleError> {
        if !matches!(property, Property::OverflowX | Property::OverflowY) {
            return self
                .geometry
                .apply(self.context, property, value, Some(self.namespace));
        }
        self.geometry.prerequisites(self.context, property, value)?;
        let axis = usize::from(property == Property::OverflowY);
        let mut candidate = self.geometry.context(self.context).style.borrow().clone();
        let parent = self.geometry.parent_style(self.context);
        candidate.overflow[axis] =
            self.geometry
                .with_relative(self.context, property, |relative| {
                    crate::style::overflow::compute_axis(
                        axis,
                        &candidate,
                        &parent,
                        relative,
                        |requested| {
                            if requested == axis {
                                return Ok(crate::style::overflow::AxisValue::Specified(
                                    value.clone(),
                                ));
                            }
                            self.geometry.property(
                                self.context,
                                [Property::OverflowX, Property::OverflowY][requested],
                            )?;
                            Ok(crate::style::overflow::AxisValue::Computed(
                                self.geometry.context(self.context).style.borrow().overflow
                                    [requested],
                            ))
                        },
                    )
                })?;
        Ok(candidate)
    }
    fn custom(&self, name: &str) -> Result<Option<variables::Expanded>, StyleError> {
        self.geometry.custom(self.context, name)
    }
    fn inherited_custom(&self, name: &str) -> Result<Option<variables::Expanded>, StyleError> {
        self.geometry.parent_custom(self.context, name)
    }
    fn substitute(
        &self,
        source: &str,
        property_syntax: bool,
    ) -> Result<Option<variables::Expanded>, StyleError> {
        let Some(value) = variables::Value::parse_context(source, 0, property_syntax)? else {
            return Ok(None);
        };
        self.geometry.substitute(self.context, &value, self)
    }
    fn scalar(&self, source: &str) -> Result<Option<conditional::Scalar>, StyleError> {
        self.geometry
            .with_relative(self.context, Property::Width, |relative| {
                conditional::Scalar::parse(source, relative)
            })
    }
    fn registered(
        &self,
        name: &str,
        registration: &Registration,
        value: &variables::Expanded,
    ) -> Result<Option<variables::Expanded>, StyleError> {
        self.geometry
            .with_relative(self.context, Property::Width, |relative| {
                crate::style::registration::compute(
                    &registration.syntax,
                    value,
                    relative,
                    self.base.as_ref(),
                    name,
                )
            })
    }
    fn registrations(&self) -> &crate::style::registration::Registrations {
        &self.geometry.sheet.registrations
    }
}

struct CustomEvaluation<'g, 'a> {
    geometry: &'g Geometry<'a>,
    context: Context,
}
impl variables::Evaluation for CustomEvaluation<'_, '_> {
    fn enter(&self, name: &str) -> Result<(), StyleError> {
        let index = self
            .geometry
            .context(self.context)
            .customs
            .names
            .binary_search_by(|candidate| candidate.as_ref().cmp(name))
            .expect("a local custom property has a declaration");
        self.geometry.enter(Node::Custom(self.context, index))
    }
    fn leave(&self, _name: &str) {
        self.geometry.active.borrow_mut().pop();
    }
    fn prepare_references(
        &self,
        _value: &variables::Value,
    ) -> Result<Option<(String, WideKeyword)>, StyleError> {
        Ok(None)
    }
    fn substitute(
        &self,
        name: &str,
        value: &variables::Value,
    ) -> Result<Option<variables::Expanded>, StyleError> {
        let owner = self
            .geometry
            .context(self.context)
            .customs
            .current(name)
            .expect("a computed custom value has a selected source");
        let base = owner.location.as_deref().map(|location| &location.base);
        let query = Query {
            geometry: self.geometry,
            context: self.context,
            namespace: owner.value.namespace,
            namespaces: owner.value.namespaces,
            base: base.cloned(),
        };
        let substituted = self.geometry.substitute(self.context, value, &query)?;
        match (self.geometry.sheet.registrations.typed(name), substituted) {
            (Some(registration), Some(substituted))
                if WideKeyword::parse(substituted.as_str()).is_none() =>
            {
                self.geometry
                    .registered(self.context, name, registration, &substituted, base)
            }
            (_, substituted) => Ok(substituted),
        }
    }
    fn recascade(
        &self,
        name: &str,
        keyword: WideKeyword,
    ) -> Result<variables::Recascade, StyleError> {
        self.geometry
            .context(self.context)
            .customs
            .recascade(name, keyword)
            .map(|source| variables::Recascade::Source(source.map(Into::into)))
    }
}

fn inherits(value: &Specified, property: Property) -> bool {
    match value {
        Specified::Wide(keyword) => keyword.resolve(property.inherited()) == Resolved::Inherit,
        Specified::Toggle(values) => values.iter().any(|value| inherits(&value.value, property)),
        _ => false,
    }
}

fn matches_parent_alignment(property: Property, value: &Specified) -> bool {
    if !matches!(property, Property::TextAlignAll | Property::TextAlignLast) {
        return false;
    }
    match value {
        Specified::Value(value) => value.keyword() == Some(Keyword::MatchParent),
        Specified::Toggle(values) => values
            .iter()
            .any(|value| matches_parent_alignment(property, &value.value)),
        Specified::FlowRelative(_, value) => matches_parent_alignment(property, value),
        _ => false,
    }
}

pub(super) fn needs_parent(property: Property, value: &Specified) -> bool {
    if let Specified::FlowRelative(_, value) = value {
        return needs_parent(property, value);
    }
    inherits(value, property)
        || matches!(value, Specified::Toggle(_))
        || matches_parent_alignment(property, value)
        || (property == Property::FontWeight
            && matches!(value, Specified::Value(value) if matches!(value.keyword(), Some(Keyword::Bolder | Keyword::Lighter))))
        || (property == Property::Color && current_color(property, value))
}

/// Whether a `<color>` depends on `currentcolor`, directly or inside a colour function.
pub(super) fn contains_current_color(value: &Component) -> bool {
    match value.unwrap() {
        Component::Keyword(keyword) => keyword
            .name()
            .eq_ignore_ascii_case(crate::style::colors::CURRENT_COLOR),
        Component::MathColor(source) => {
            let mut pending = vec![source.as_ref()];
            while let Some(source) = pending.pop() {
                let mut raw = cssparser::ParserInput::new(source);
                let input = &mut cssparser::Parser::new(&mut raw);
                while let Ok(token) = input.next().cloned() {
                    match token {
                        cssparser::Token::Ident(name)
                            if name.eq_ignore_ascii_case(crate::style::colors::CURRENT_COLOR) =>
                        {
                            return true;
                        }
                        cssparser::Token::Function(_) | cssparser::Token::ParenthesisBlock => {
                            if let Ok(source) = input.parse_nested_block(|nested| {
                                Ok::<_, cssparser::ParseError<'_, ()>>(
                                    crate::style::conditions::block_source(nested),
                                )
                            }) {
                                pending.push(source);
                            }
                        }
                        _ => {}
                    }
                }
            }
            false
        }
        Component::Group(values) => values.iter().any(contains_current_color),
        Component::Slots(values) => values.iter().flatten().any(contains_current_color),
        Component::Function(_, Some(value)) | Component::Block(_, value) => {
            contains_current_color(value)
        }
        _ => false,
    }
}

fn current_color(property: Property, value: &Specified) -> bool {
    match value {
        Specified::Value(value) => contains_current_color(value),
        Specified::Wide(keyword) if keyword.resolve(property.inherited()) == Resolved::Initial => {
            crate::style::declaration::initial(property).is_some_and(contains_current_color)
        }
        Specified::Toggle(values) => values
            .iter()
            .any(|value| current_color(property, &value.value)),
        Specified::FlowRelative(_, value) => current_color(property, value),
        _ => false,
    }
}

pub(super) struct ValueContext<'a> {
    pub tree: Tree<'a>,
    pub viewport: [f32; 2],
    pub page_font: bool,
}

pub(super) fn resolve_values(
    sheet: &Stylesheet,
    mut style: Style,
    parent: &Style,
    values: Cascaded,
    resolver: &dyn ResourceResolver,
    context: &ValueContext<'_>,
) -> Result<(Style, Winners), StyleError> {
    if !values.contextual(&sheet.registrations) {
        let winners = crate::style::cascade::resolve(values, &mut style, parent)?;
        return Ok((style, winners));
    }
    let graph = Geometry {
        sheet,
        page_rules: OnceCell::new(),
        document: None,
        select_first: None,
        select_side: None,
        first_request: RefCell::new(sheet.first_page.clone()),
        selection_done: Cell::new(false),
        side_selection_done: Cell::new(false),
        tree_contexts: RefCell::default(),
        viewport: Some(context.viewport),
        resolver,
        root: Rc::new(Computed::provided(Style::default())?),
        page: OnceCell::from(Rc::new(Computed::provided(Style::default())?)),
        element: Some(Rc::new(Computed::new(style, values)?)),
        parent: Some(parent),
        tree: context.tree,
        page_font: context.page_font,
        provided_root: true,
        provided_element: false,
        size: OnceCell::new(),
        size_source: RefCell::default(),
        complete: RefCell::default(),
        dimensions: RefCell::default(),
        active: RefCell::default(),
        axes: RefCell::default(),
        work: Rc::default(),
    };
    graph.materialized(Context::Element)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the declaration's namespaces and base, the page context and its resources are independent inputs"
)]
pub(super) fn substitute_known(
    sheet: &Stylesheet,
    style: &Style,
    value: &variables::Value,
    namespace: crate::fonts::FontNamespace,
    namespaces: Option<&crate::style::selector::Namespaces>,
    base: Option<&crate::resources::ResourceBase>,
    viewport: [f32; 2],
    resolver: &dyn ResourceResolver,
) -> Result<Option<variables::Expanded>, StyleError> {
    if !value.requires_context() {
        return value.substitute(&style.custom_properties, &style.inherited_custom_properties);
    }
    let parent = style.page_parent.as_deref().ok_or_else(|| {
        StyleError::Invalid(
            "conditional page descriptors require their computed page parent".into(),
        )
    })?;
    let graph = Geometry {
        sheet,
        page_rules: OnceCell::new(),
        document: None,
        select_first: None,
        select_side: None,
        first_request: RefCell::new(sheet.first_page.clone()),
        selection_done: Cell::new(false),
        side_selection_done: Cell::new(false),
        tree_contexts: RefCell::default(),
        viewport: Some(viewport),
        resolver,
        root: Rc::new(Computed::provided(Style::default())?),
        page: OnceCell::from(Rc::new(Computed::provided(Style::default())?)),
        element: Some(Rc::new(Computed::provided(style.clone())?)),
        parent: Some(parent),
        tree: Tree::Root,
        page_font: true,
        provided_root: true,
        provided_element: true,
        size: OnceCell::new(),
        size_source: RefCell::default(),
        complete: RefCell::default(),
        dimensions: RefCell::default(),
        active: RefCell::default(),
        axes: RefCell::default(),
        work: Rc::default(),
    };
    graph.substitute(
        Context::Element,
        value,
        &Query {
            geometry: &graph,
            context: Context::Element,
            namespace,
            namespaces: namespaces.cloned(),
            base: base.cloned(),
        },
    )
}
