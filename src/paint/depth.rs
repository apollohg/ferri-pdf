//! CSS Transforms 2 §4.1.2 depth sorting of the planes of a 3D rendering context: each plane is
//! cut where others pass through it and the pieces order back to front (a binary space
//! partition, which orders what Newell's algorithm orders), coplanar pieces in painting order.
use crate::paint::{
    Rect,
    projection::{self, Line},
};

/// Distance in points within which a piece lies in a plane, where floating-point noise in the
/// composed matrices would otherwise split coplanar planes.
const COPLANAR_TOLERANCE_PT: f64 = 0.01;
const DIMENSION: usize = 4;

/// A piece of a plane: the part of the plane's content within `polygon`, in its own space.
pub(crate) struct Piece {
    pub(crate) plane: usize,
    pub(crate) polygon: Vec<[f32; 2]>,
    /// Whether a partition cut the piece out of its plane's content.
    pub(crate) cut: bool,
}

/// The pieces of planes, each a column-major accumulated 3D matrix with the bounds of its
/// content in its own space, if any, back to front.
pub(crate) fn back_to_front(planes: &[([f32; 16], Option<Rect>)]) -> Vec<Piece> {
    let pieces = planes
        .iter()
        .enumerate()
        .filter_map(|(index, (matrix, ink))| {
            let polygon = projection::clip(
                &projection::corners((*ink)?),
                projection::front(projection::of_plane(*matrix)),
            );
            (polygon.len() >= 3).then_some(Piece {
                plane: index,
                polygon,
                cut: false,
            })
        })
        .collect();
    let matrices: Vec<[f32; 16]> = planes.iter().map(|(matrix, _)| *matrix).collect();
    let mut ordered = Vec::with_capacity(planes.len());
    partition(&matrices, pieces, &mut ordered);
    ordered
}

/// The homogeneous plane `[a, b, c, d]`, `a·X + b·Y + c·Z + d·W = 0`, through every point of
/// the plane matrix `m` places: orthogonal to its x, y and translation columns.
fn equation(m: [f32; 16]) -> [f64; 4] {
    let column = |index: usize| -> [f64; 4] {
        std::array::from_fn(|row| f64::from(m[index * DIMENSION + row]))
    };
    let rows = [column(0), column(1), column(3)];
    std::array::from_fn(|axis| {
        let others: Vec<usize> = (0..DIMENSION).filter(|&other| other != axis).collect();
        let minor = |row: usize, column: usize| rows[row][others[column]];
        let determinant = minor(0, 0) * (minor(1, 1) * minor(2, 2) - minor(1, 2) * minor(2, 1))
            - minor(0, 1) * (minor(1, 0) * minor(2, 2) - minor(1, 2) * minor(2, 0))
            + minor(0, 2) * (minor(1, 0) * minor(2, 1) - minor(1, 1) * minor(2, 0));
        if axis % 2 == 0 {
            determinant
        } else {
            -determinant
        }
    })
}

/// `plane` as a function of the local points of the plane matrix `m` places: its sign is the
/// side of `plane` each lies on, where `w` is positive.
fn pulled(plane: [f64; 4], m: [f32; 16]) -> Line {
    let along = |index: usize| -> f64 {
        (0..DIMENSION)
            .map(|row| plane[row] * f64::from(m[index * DIMENSION + row]))
            .sum()
    };
    [along(0), along(1), along(3)]
}

/// Orders `pieces` of the planes `matrices` place back to front into `ordered`: the first
/// piece's plane splits the others, and the side away from the viewer, who looks down the z
/// axis from far along it, paints first.
fn partition(matrices: &[[f32; 16]], mut pieces: Vec<Piece>, ordered: &mut Vec<Piece>) {
    if pieces.is_empty() {
        return;
    }
    let splitter = pieces.remove(0);
    let plane = equation(matrices[splitter.plane]);
    let scale = plane[0].hypot(plane[1]).hypot(plane[2]);
    let mut coplanar = vec![splitter];
    let (mut front, mut back) = (Vec::new(), Vec::new());
    for piece in pieces {
        let m = matrices[piece.plane];
        let line = pulled(plane, m);
        let w = projection::front(projection::of_plane(m));
        let distances: Vec<f64> = piece
            .polygon
            .iter()
            .map(|&point| projection::side(line, point) / (projection::side(w, point) * scale))
            .collect();
        let ahead = distances
            .iter()
            .any(|&distance| distance > COPLANAR_TOLERANCE_PT);
        let behind = distances
            .iter()
            .any(|&distance| distance < -COPLANAR_TOLERANCE_PT);
        match (ahead, behind) {
            (false, false) => coplanar.push(piece),
            (true, false) => front.push(piece),
            (false, true) => back.push(piece),
            (true, true) => {
                for (side, sign) in [(&mut front, 1.0), (&mut back, -1.0)] {
                    let polygon = projection::clip(&piece.polygon, line.map(|value| sign * value));
                    if polygon.len() >= 3 {
                        side.push(Piece {
                            plane: piece.plane,
                            polygon,
                            cut: true,
                        });
                    }
                }
            }
        }
    }
    coplanar.sort_by_key(|piece| piece.plane);
    let (far, near) = if plane[2] >= 0.0 {
        (back, front)
    } else {
        (front, back)
    };
    partition(matrices, far, ordered);
    ordered.extend(coplanar);
    partition(matrices, near, ordered);
}
