const DEGREES_PER_GRADIAN: f64 = 0.9;
const DEGREES_PER_RADIAN: f64 = 180.0 / std::f64::consts::PI;
const DEGREES_PER_TURN: f64 = 360.0;
const ANGLE_UNITS: [(&str, f64); 4] = [
    ("deg", 1.0),
    ("grad", DEGREES_PER_GRADIAN),
    ("rad", DEGREES_PER_RADIAN),
    ("turn", DEGREES_PER_TURN),
];

pub(crate) fn degrees_per_unit(unit: &str) -> Option<f64> {
    ANGLE_UNITS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(unit))
        .map(|(_, degrees)| *degrees)
}
