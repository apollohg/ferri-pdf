// Copyright 2020 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::ImageRefMut;
use crate::svg::usvg::filter::{ComponentTransfer, TransferFunction};

/// Applies component transfer functions for each `src` image channel.
///
/// Input image pixels should have an **unpremultiplied alpha**.
pub(super) fn apply(fe: &ComponentTransfer, src: ImageRefMut) {
    for pixel in src.data {
        if !is_dummy(fe.func_r()) {
            pixel.r = transfer(fe.func_r(), pixel.r);
        }

        if !is_dummy(fe.func_b()) {
            pixel.b = transfer(fe.func_b(), pixel.b);
        }

        if !is_dummy(fe.func_g()) {
            pixel.g = transfer(fe.func_g(), pixel.g);
        }

        if !is_dummy(fe.func_a()) {
            pixel.a = transfer(fe.func_a(), pixel.a);
        }
    }
}

fn is_dummy(func: &TransferFunction) -> bool {
    match func {
        TransferFunction::Identity => true,
        TransferFunction::Table(values) => values.is_empty(),
        TransferFunction::Discrete(values) => values.is_empty(),
        TransferFunction::Linear { .. } => false,
        TransferFunction::Gamma { .. } => false,
    }
}

fn transfer(func: &TransferFunction, c: u8) -> u8 {
    let c = c as f32 / 255.0;
    let c = func.evaluate(c);

    (c * f32::from(u8::MAX)) as u8
}
