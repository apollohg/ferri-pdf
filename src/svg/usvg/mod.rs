// Copyright 2018 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/*!
`usvg` (micro SVG) is an [SVG] parser that tries to solve most of SVG complexity.

SVG is notoriously hard to parse. `usvg` presents a layer between an XML library and
a potential SVG rendering library. It will parse an input SVG into a strongly-typed tree structure
were all the elements, attributes, references and other SVG features are already resolved
and presented in the simplest way possible.
So a caller doesn't have to worry about most of the issues related to SVG parsing
and can focus just on the rendering part.

## Features

- All supported attributes are resolved.
  No need to worry about inheritable, implicit and default attributes
- CSS will be applied
- Only simple paths
  - Basic shapes (like `rect` and `circle`) will be converted into paths
  - Paths contain only absolute *MoveTo*, *LineTo*, *QuadTo*, *CurveTo* and *ClosePath* segments.
    ArcTo, implicit and relative segments will be converted
- `use` will be resolved and replaced with the reference content
- Nested `svg` will be resolved
- Invalid, malformed elements will be removed
- Relative length units (mm, em, etc.) will be converted into pixels/points
- External images will be loaded
- Internal, base64 images will be decoded
- All references (like `#elem` and `url(#elem)`) will be resolved
- `switch` will be resolved
- Text elements, which are probably the hardest part of SVG, will be completely resolved.
  This includes all the attributes resolving, whitespaces preprocessing (`xml:space`),
  text chunks and spans resolving
- Markers will be converted into regular elements. No need to place them manually
- All filters are supported. Including filter functions, like `filter="contrast(50%)"`
- Recursive elements will be detected and removed
- `objectBoundingBox` will be replaced with `userSpaceOnUse`

## Limitations

- Unsupported SVG features will be ignored
- CSS support is minimal
- Only [static](http://www.w3.org/TR/SVG11/feature#SVG-static) SVG features,
  e.g. no `a`, `view`, `cursor`, `script`, no events and no animations

[SVG]: https://en.wikipedia.org/wiki/Scalable_Vector_Graphics
*/

#![allow(
    clippy::pedantic,
    clippy::allow_attributes_without_reason,
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::derivable_impls,
    clippy::enum_variant_names,
    clippy::identity_op,
    clippy::if_same_then_else,
    clippy::large_enum_variant,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::op_ref,
    clippy::option_map_unit_fn,
    clippy::question_mark,
    clippy::replace_box,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unnecessary_map_or,
    clippy::upper_case_acronyms,
    clippy::while_let_loop,
    clippy::wrong_self_convention,
    elided_lifetimes_in_paths,
    reason = "inlined upstream code keeps its upstream style"
)]

mod parser;
mod text;
mod tree;

pub(crate) use parser::*;
pub(crate) use text::*;
pub(crate) use tree::*;
