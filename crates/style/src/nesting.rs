use crate::{
    StyleError,
    parser::{NestedRule, RuleSource},
    selector::RuleSelector,
};
use std::rc::Rc;

pub(super) struct PreparedRule {
    pub selectors: Rc<[RuleSelector]>,
    pub source: RuleSource,
}

pub(super) fn prepare_nested(
    parent: &[RuleSelector],
    rule: &NestedRule,
) -> Result<Vec<PreparedRule>, StyleError> {
    let selectors = crate::selector::resolve_selectors(
        if rule.nesting { parent } else { &[] },
        &rule.selectors,
    )?;
    Ok(vec![PreparedRule {
        selectors: selectors.into(),
        source: rule.source.clone(),
    }])
}
