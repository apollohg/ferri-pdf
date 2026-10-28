#[derive(Clone, Copy, Default)]
pub(super) struct Variable {
    pub min: f32,
    pub max: f32,
    pub specified: Option<f32>,
    pub generated: bool,
}
impl Variable {
    fn bounds(self) -> [f32; 2] {
        if !self.generated {
            [0.0; 2]
        } else if let Some(value) = self.specified {
            [value; 2]
        } else {
            [self.min, self.max]
        }
    }
    fn auto(self) -> bool {
        self.generated && self.specified.is_none()
    }
}
fn pair(a: Variable, b: Variable, available: f32) -> [f32; 2] {
    let [amin, amax] = a.bounds();
    let [bmin, bmax] = b.bounds();
    match (a.auto(), b.auto()) {
        (false, false) => [amax, bmax],
        (true, false) => [(available - bmax), bmax],
        (false, true) => [amax, (available - amax)],
        (true, true) => {
            let (base, mut factors) = if amax + bmax < available {
                ([amax, bmax], [amax, bmax])
            } else if amin + bmin < available {
                ([amin, bmin], [amax - amin, bmax - bmin])
            } else {
                ([amin, bmin], [amin, bmin])
            };
            if factors[0] + factors[1] == 0.0 {
                factors = [1.0; 2];
            }
            let flex = available - base[0] - base[1];
            [
                base[0] + flex * factors[0] / (factors[0] + factors[1]),
                base[1] + flex * factors[1] / (factors[0] + factors[1]),
            ]
        }
    }
}
pub(super) fn variable(boxes: [Variable; 3], available: f32) -> [f32; 3] {
    let [a, b, c] = boxes;
    if !b.generated {
        let [a, c] = pair(a, c, available);
        return [a, 0.0, c];
    }
    let [amin, amax] = a.bounds();
    let [cmin, cmax] = c.bounds();
    let combined = Variable {
        min: 2.0 * amin.max(cmin),
        max: 2.0 * amax.max(cmax),
        specified: if a.specified.is_some() || c.specified.is_some() {
            Some(2.0 * amin.max(cmin))
        } else if !a.generated && !c.generated {
            Some(0.0)
        } else {
            None
        },
        generated: a.generated || c.generated,
    };
    let middle = pair(b, combined, available)[0];
    let side = ((available - middle) / 2.0).max(0.0);
    [
        if a.generated { side } else { 0.0 },
        middle,
        if c.generated { side } else { 0.0 },
    ]
}
