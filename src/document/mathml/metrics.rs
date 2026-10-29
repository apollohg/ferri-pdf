//! `MathML` Core §5.1: MATH constants and the specified non-MATH fallbacks.
use crate::fonts::{FontBook, FontError};
use crate::resources::ResourceResolver;
use crate::style::Style;

pub(super) struct Metrics {
    pub axis: f32,
    pub rule: f32,
    pub numerator: [f32; 2],
    pub denominator: [f32; 2],
    pub numerator_gap: [f32; 2],
    pub denominator_gap: [f32; 2],
    pub stack_top: [f32; 2],
    pub stack_bottom: [f32; 2],
    pub stack_gap: [f32; 2],
    pub sub_shift: f32,
    pub sub_top: f32,
    pub sub_drop: f32,
    pub sup_shift: [f32; 2],
    pub sup_bottom: f32,
    pub sup_drop: f32,
    pub script_gap: f32,
    pub sup_bottom_with_sub: f32,
    pub after_script: f32,
}
impl Metrics {
    pub(super) fn of(
        fonts: &FontBook,
        style: &Style,
        resolver: &dyn ResourceResolver,
    ) -> Result<Self, FontError> {
        for &id in style.font_candidates() {
            let Some((instance, size)) = fonts.sized_instance(
                id,
                style.font_selection(),
                &style.font_adjustments,
                style.size,
                resolver,
            )?
            else {
                continue;
            };
            let face = &instance.face;
            let scale = size / f32::from(face.units_per_em());
            let constants = face.tables().math.and_then(|math| math.constants);
            let x_height = f32::from(face.x_height().unwrap_or(0)) * scale;
            let default_rule = face
                .underline_metrics()
                .map_or(0.0, |metrics| f32::from(metrics.thickness) * scale);
            macro_rules! value {
                ($name:ident, $fallback:expr) => {
                    constants.map_or($fallback, |table| f32::from(table.$name().value) * scale)
                };
            }
            const SUBSCRIPT_TOP_RATIO: f32 = 4.0 / 5.0;
            const SUPERSCRIPT_BOTTOM_RATIO: f32 = 1.0 / 4.0;
            const SCRIPT_SPACE_EM: f32 = 1.0 / 24.0;
            const SCRIPT_GAP_RULES: f32 = 4.0;
            const STACK_GAP_RULES: f32 = 3.0;
            const DISPLAY_STACK_GAP_RULES: f32 = 7.0;
            const DISPLAY_FRACTION_GAP_RULES: f32 = 3.0;
            return Ok(Self {
                axis: value!(axis_height, x_height / 2.0),
                rule: value!(fraction_rule_thickness, default_rule),
                numerator: [
                    value!(fraction_numerator_display_style_shift_up, 0.0),
                    value!(fraction_numerator_shift_up, 0.0),
                ],
                denominator: [
                    value!(fraction_denominator_display_style_shift_down, 0.0),
                    value!(fraction_denominator_shift_down, 0.0),
                ],
                numerator_gap: [
                    value!(
                        fraction_num_display_style_gap_min,
                        DISPLAY_FRACTION_GAP_RULES * default_rule
                    ),
                    value!(fraction_numerator_gap_min, default_rule),
                ],
                denominator_gap: [
                    value!(
                        fraction_denom_display_style_gap_min,
                        DISPLAY_FRACTION_GAP_RULES * default_rule
                    ),
                    value!(fraction_denominator_gap_min, default_rule),
                ],
                stack_top: [
                    value!(stack_top_display_style_shift_up, 0.0),
                    value!(stack_top_shift_up, 0.0),
                ],
                stack_bottom: [
                    value!(stack_bottom_display_style_shift_down, 0.0),
                    value!(stack_bottom_shift_down, 0.0),
                ],
                stack_gap: [
                    value!(
                        stack_display_style_gap_min,
                        DISPLAY_STACK_GAP_RULES * default_rule
                    ),
                    value!(stack_gap_min, STACK_GAP_RULES * default_rule),
                ],
                sub_shift: value!(
                    subscript_shift_down,
                    face.subscript_metrics()
                        .map_or(0.0, |metrics| f32::from(metrics.y_offset) * scale)
                ),
                sub_top: value!(subscript_top_max, SUBSCRIPT_TOP_RATIO * x_height),
                sub_drop: value!(subscript_baseline_drop_min, 0.0),
                sup_shift: [
                    value!(
                        superscript_shift_up,
                        face.superscript_metrics()
                            .map_or(0.0, |metrics| f32::from(metrics.y_offset) * scale)
                    ),
                    value!(superscript_shift_up_cramped, 0.0),
                ],
                sup_bottom: value!(superscript_bottom_min, SUPERSCRIPT_BOTTOM_RATIO * x_height),
                sup_drop: value!(superscript_baseline_drop_max, 0.0),
                script_gap: value!(sub_superscript_gap_min, SCRIPT_GAP_RULES * default_rule),
                sup_bottom_with_sub: value!(
                    superscript_bottom_max_with_subscript,
                    SUBSCRIPT_TOP_RATIO * x_height
                ),
                after_script: value!(space_after_script, SCRIPT_SPACE_EM * size),
            });
        }
        Err(FontError::NoAvailableFace)
    }
}
