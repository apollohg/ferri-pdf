use crate::dom::{Element, SUBJECT_FEATURES, subject_features};
use crate::style::selector::RuleSelector;
use std::collections::{HashMap, HashSet};

/// The style rules with a selector of one target, bucketed by the ID, class or local name
/// their subject compounds require, as Servo's `SelectorMap` does, so that an element tries
/// only the rules of the buckets its own features name and the rules that require none.
#[derive(Clone, Default)]
pub(super) struct RuleMap {
    features: [HashMap<Box<str>, Vec<usize>>; SUBJECT_FEATURES],
    universal: Vec<usize>,
    /// The `@container` rules that hold a rule of the map, in the order of their first rule.
    containers: Vec<usize>,
    held_containers: HashSet<usize>,
}
impl RuleMap {
    /// Adds rule `index`, which `selector` of it targets and which `container` holds; rules
    /// are added in cascade order.
    pub(super) fn insert(
        &mut self,
        index: usize,
        selector: &RuleSelector,
        container: Option<usize>,
    ) {
        let rules = match selector.subject_feature() {
            Some((feature, name)) => self.features[feature as usize].entry(name).or_default(),
            None => &mut self.universal,
        };
        if rules.last() != Some(&index) {
            rules.push(index);
        }
        if let Some(container) = container
            && self.held_containers.insert(container)
        {
            self.containers.push(container);
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.universal.is_empty() && self.features.iter().all(HashMap::is_empty)
    }

    pub(super) fn containers(&self) -> &[usize] {
        &self.containers
    }

    /// The rules whose selectors can match `element`, in cascade order.
    pub(super) fn candidates(&self, element: Element<'_>) -> Vec<usize> {
        let mut candidates = self.universal.clone();
        subject_features(element, |feature, name| {
            if let Some(rules) = self.features[feature as usize].get(name) {
                candidates.extend_from_slice(rules);
            }
        });
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}
