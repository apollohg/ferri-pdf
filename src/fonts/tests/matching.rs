use crate::fonts::{
    FontBook, FontDescriptor, FontFaceStyle, FontFamily, FontRange, FontSelection, FontSlant,
    FontStyle, FontWeight, FontWidth, GenericFamily,
};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSans.ttf");
const VARIABLE_FONT: &[u8] = include_bytes!("../../../tests/fixtures/fonts/RobotoFlex.ttf");

#[test]
fn variable_faces_retain_the_original_data_for_shaping_and_embedding() {
    let mut book = FontBook::default();
    let registered = book.register(
        FontDescriptor {
            family: "Variable Fixture".into(),
            weight: FontRange::single(FontWeight::NORMAL),
            width: FontRange::single(FontWidth::NORMAL),
            style: FontFaceStyle::Normal,
        },
        VARIABLE_FONT.to_vec(),
    );
    assert!(
        registered.is_ok(),
        "a valid variable TrueType face must be usable: {registered:?}"
    );
    let id = registered.unwrap();
    assert!(
        book.face(id).unwrap().data() == Some(VARIABLE_FONT),
        "the original variable face must remain available to both shaper and PDF embedder"
    );
}

#[test]
fn relative_weight_extremes_preserve_values_beyond_the_named_weights() {
    let very_light = FontWeight::new(50.0).unwrap();
    let very_heavy = FontWeight::new(950.0).unwrap();
    assert_eq!(
        very_light.lighter(),
        very_light,
        "lighter below 100 must leave the inherited weight unchanged"
    );
    assert_eq!(
        very_heavy.bolder(),
        very_heavy,
        "bolder above 900 must leave the inherited weight unchanged"
    );
}

#[test]
fn matching_uses_css_search_order_and_last_matching_author_face() {
    let mut book = FontBook::default();
    let family = "Weighted";
    let mut ids = Vec::new();
    for weight in [300.0, 500.0, 600.0, 500.0] {
        ids.push(
            book.register(
                FontDescriptor {
                    family: family.into(),
                    weight: FontRange::single(FontWeight::new(weight).unwrap()),
                    width: FontRange::single(FontWidth::NORMAL),
                    style: FontFaceStyle::Normal,
                },
                FONT.to_vec(),
            )
            .unwrap(),
        );
    }
    let families = [FontFamily::Named(family.into())];
    for (requested, expected) in [
        (400.0, ids[3]),
        (450.0, ids[3]),
        (350.0, ids[0]),
        (550.0, ids[2]),
        (650.0, ids[2]),
    ] {
        assert_eq!(
            book.candidates(
                &families,
                FontSelection::new(FontWeight::new(requested).unwrap(), FontStyle::Normal)
            )[0],
            expected,
            "font-weight {requested} must search in the CSS-prescribed direction and prefer later equivalent faces"
        );
    }
}

fn family_order(book: &FontBook, families: &[FontFamily], style: FontStyle) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    for &id in book
        .candidates(families, FontSelection::new(FontWeight::NORMAL, style))
        .iter()
    {
        let family = &book.face(id).unwrap().descriptor().family;
        if order.last() != Some(family) {
            order.push(family.clone());
        }
    }
    order
}

#[test]
fn fallback_tries_metric_compatible_generic_faces_before_wider_dejavu_coverage() {
    let mut book = FontBook::default();
    let author = "Author Face";
    book.register(
        FontDescriptor {
            family: author.into(),
            weight: FontRange::single(FontWeight::NORMAL),
            width: FontRange::single(FontWidth::NORMAL),
            style: FontFaceStyle::Normal,
        },
        FONT.to_vec(),
    )
    .unwrap();
    let named = FontFamily::Named(author.into());
    let sans = FontFamily::Generic(GenericFamily::SansSerif);
    let mono = FontFamily::Generic(GenericFamily::Monospace);
    for (families, style, expected) in [
        (
            vec![named.clone(), sans],
            FontStyle::Normal,
            [
                author,
                "Liberation Sans",
                "DejaVu Sans",
                "Liberation Serif",
                "DejaVu Serif",
                "Liberation Mono",
                "DejaVu Sans Mono",
            ],
        ),
        (
            vec![named.clone()],
            FontStyle::Normal,
            [
                author,
                "Liberation Serif",
                "DejaVu Serif",
                "Liberation Sans",
                "DejaVu Sans",
                "Liberation Mono",
                "DejaVu Sans Mono",
            ],
        ),
        (
            vec![mono, named],
            FontStyle::Italic,
            [
                "Liberation Mono",
                "DejaVu Sans Mono",
                author,
                "Liberation Serif",
                "DejaVu Serif",
                "Liberation Sans",
                "DejaVu Sans",
            ],
        ),
    ] {
        assert_eq!(
            family_order(&book, &families, style),
            expected,
            "{families:?} ({style:?}): a generic expands to its Arial/Times/Courier New-compatible face, then DejaVu; glyphs missing from the list use the initial serif family, then every other bundled family"
        );
    }
}

#[test]
fn overlapping_weight_ranges_follow_the_400_to_500_search_direction() {
    let mut book = FontBook::default();
    let families = [FontFamily::Named("Weight ranges".into())];
    let ids: Vec<_> = [(100.0, 430.0), (700.0, 480.0), (600.0, 900.0)]
        .into_iter()
        .map(|(start, end)| {
            book.register(
                FontDescriptor {
                    family: "Weight ranges".into(),
                    weight: FontRange::new(
                        FontWeight::new(start).unwrap(),
                        FontWeight::new(end).unwrap(),
                    ),
                    width: FontRange::single(FontWidth::NORMAL),
                    style: FontFaceStyle::Normal,
                },
                FONT.to_vec(),
            )
            .unwrap()
        })
        .collect();
    for (requested, expected) in [
        (400.0, ids[0]),
        (450.0, ids[1]),
        (550.0, ids[1]),
        (750.0, ids[2]),
        (50.0, ids[0]),
    ] {
        let actual = book.candidates(
            &families,
            FontSelection::new(FontWeight::new(requested).unwrap(), FontStyle::Normal),
        );
        assert_eq!(
            actual[0], expected,
            "weight {requested} must search the inclusive ranges in the prescribed direction, including reversed endpoints"
        );
    }
}

#[test]
fn signed_oblique_ranges_respect_the_eleven_degree_preference_boundary() {
    let mut book = FontBook::default();
    let families = [FontFamily::Named("Slant ranges".into())];
    let ids: Vec<_> = [5.0, 20.0, -5.0, -20.0]
        .into_iter()
        .map(|angle| {
            book.register(
                FontDescriptor {
                    family: "Slant ranges".into(),
                    weight: FontRange::single(FontWeight::NORMAL),
                    width: FontRange::single(FontWidth::NORMAL),
                    style: FontFaceStyle::Oblique(FontRange::single(
                        FontSlant::new(angle).unwrap(),
                    )),
                },
                FONT.to_vec(),
            )
            .unwrap()
        })
        .collect();
    for (angle, expected) in [
        (10.0, ids[0]),
        (14.0, ids[1]),
        (-10.0, ids[2]),
        (-14.0, ids[3]),
    ] {
        let selection = FontSelection {
            slant: FontSlant::new(angle).unwrap(),
            ..FontSelection::new(FontWeight::NORMAL, FontStyle::Oblique)
        };
        assert_eq!(
            book.candidates(&families, selection)[0],
            expected,
            "oblique {angle}deg must prefer the correct sign and search direction"
        );
    }
}

#[test]
fn caseless_family_aliases_do_not_switch_descriptors_after_missing_glyphs() {
    use crate::fonts::{FontError, GlyphMatch};
    const LIMITED: &[u8] = include_bytes!("../../../tests/fixtures/fonts/MetricGlyphTest.ttf");
    let mut book = FontBook::default();
    let descriptor = FontDescriptor {
        family: "Straße".into(),
        weight: FontRange::single(FontWeight::NORMAL),
        width: FontRange::single(FontWidth::NORMAL),
        style: FontFaceStyle::Italic,
    };
    let selected = book.register(descriptor.clone(), LIMITED.to_vec()).unwrap();
    let alternate = book
        .register(
            FontDescriptor {
                family: "STRASSE".into(),
                style: FontFaceStyle::Normal,
                ..descriptor.clone()
            },
            FONT.to_vec(),
        )
        .unwrap();
    let fallback = book
        .register(
            FontDescriptor {
                family: "Other Family".into(),
                ..descriptor
            },
            FONT.to_vec(),
        )
        .unwrap();
    let mut probed = Vec::new();
    let found: Result<_, FontError> = book.find_glyph(&[selected, alternate, fallback], |id| {
        probed.push(id);
        let face = ttf_parser::Face::parse(book.face(id).unwrap().data().unwrap(), 0).unwrap();
        Ok(if face.glyph_index('H').is_some() {
            GlyphMatch::Found(())
        } else {
            GlyphMatch::Missing
        })
    });
    assert_eq!(
        found.unwrap().map(|(id, ())| id),
        Some(fallback),
        "missing H must continue to the next family, not change style through a caseless alias"
    );
    assert_eq!(
        probed,
        [selected, fallback],
        "the alternate descriptor of the same folded family must never be probed"
    );
}

#[test]
fn generic_preferences_select_installed_faces_with_the_requested_style() {
    let book = FontBook::with_preferences(crate::fonts::FontPreferences {
        sans_serif: vec!["DejaVu Sans".into()],
        serif: vec!["DejaVu Serif".into()],
        monospace: vec!["DejaVu Sans Mono".into()],
        ..Default::default()
    });
    for (generic, name, data) in [
        (
            GenericFamily::SansSerif,
            "DejaVu Sans",
            include_bytes!("../../../assets/fonts/DejaVuSans-BoldOblique.ttf").as_slice(),
        ),
        (
            GenericFamily::Serif,
            "DejaVu Serif",
            include_bytes!("../../../assets/fonts/DejaVuSerif-BoldItalic.ttf").as_slice(),
        ),
        (
            GenericFamily::Monospace,
            "DejaVu Sans Mono",
            include_bytes!("../../../assets/fonts/DejaVuSansMono-BoldOblique.ttf").as_slice(),
        ),
    ] {
        let candidates = book.candidates(
            &[FontFamily::Generic(generic)],
            FontSelection::new(FontWeight::BOLD, FontStyle::Italic),
        );
        let face = book.face(candidates[0]).unwrap();
        assert_eq!(
            face.descriptor().family,
            name,
            "configured {generic:?} must select its own family"
        );
        assert_eq!(
            face.data(),
            Some(data),
            "{generic:?} must select the actual bold italic/oblique font"
        );
    }
}

#[test]
fn preferred_fallback_follows_the_css_family_list_and_deduplicates_faces() {
    let book = FontBook::with_preferences(crate::fonts::FontPreferences {
        sans_serif: vec![
            "Unavailable Family".into(),
            "DejaVu ſans".into(),
            "DejaVu Sans".into(),
        ],
        fallback: vec![
            "DejaVu Sans Mono".into(),
            "DejaVu Serif".into(),
            "DejaVu Sans Mono".into(),
        ],
        ..Default::default()
    });
    let named = [FontFamily::Named("Liberation Serif".into())];
    let order = family_order(&book, &named, FontStyle::Normal);
    assert_eq!(
        &order[..3],
        ["Liberation Serif", "DejaVu Sans Mono", "DejaVu Serif"],
        "the complete authored list precedes the installed fallback preference"
    );
    let families = [FontFamily::Generic(GenericFamily::SansSerif)];
    let candidates = book.candidates(&families, FontSelection::default());
    assert_eq!(
        book.face(candidates[0]).unwrap().descriptor().family,
        "DejaVu Sans"
    );
    for id in candidates.iter() {
        assert_eq!(
            candidates.iter().filter(|other| *other == id).count(),
            1,
            "aliases and repeated preferences must not repeat font probing"
        );
    }
}

#[test]
fn generic_preferences_cannot_turn_an_authored_face_into_installed_fallback() {
    let mut book = FontBook::with_preferences(crate::fonts::FontPreferences {
        sans_serif: vec!["DejaVu Sans".into()],
        fallback: vec!["Declared Only".into()],
        ..Default::default()
    });
    let descriptor = |family: &str| FontDescriptor {
        family: family.into(),
        weight: FontRange::single(FontWeight::NORMAL),
        width: FontRange::single(FontWidth::NORMAL),
        style: FontFaceStyle::Normal,
    };
    let sources = || crate::fonts::FontSources {
        sources: vec![crate::fonts::FontSource::Url("denied-font.ttf".into())],
        base: Default::default(),
    };
    let shadow = book.declare(descriptor("DejaVu Sans"), sources());
    let declared = book.declare(descriptor("Declared Only"), sources());
    let generic = [FontFamily::Generic(GenericFamily::SansSerif)];
    let candidates = book.candidates(&generic, FontSelection::default());
    assert!(
        !candidates.contains(&shadow),
        "generic preferences use installed fonts, not CSS aliases"
    );
    assert!(
        !candidates.contains(&declared),
        "installed fallback cannot expose an unrequested webfont"
    );
    let named = [FontFamily::Named("DejaVu Sans".into())];
    assert_eq!(
        book.candidates(&named, FontSelection::default())[0],
        shadow,
        "explicitly named authored families keep their ordinary precedence"
    );
    assert!(
        book.named_candidates_in_namespace(
            crate::fonts::FontNamespace::ROOT,
            &generic,
            FontSelection::default()
        )
        .is_empty(),
        "private-use matching must still require CSS named families"
    );
}

#[test]
fn cloned_books_share_lazy_results_and_original_bytes_without_acquiring_cached_faces() {
    use crate::fonts::{FontError, FontSource, FontSources};
    use crate::resources::{Resource, ResourceBase, ResourceError, ResourceResolver};
    use std::cell::Cell;
    struct Assets {
        calls: Cell<usize>,
    }
    impl ResourceResolver for Assets {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.calls.set(self.calls.get() + 1);
            if reference == "missing.ttf" {
                return Err(ResourceError::UnsupportedScheme(reference.into()));
            }
            assert_eq!(reference, "face.ttf");
            Ok(Resource {
                bytes: FONT.to_vec(),
                base: base.clone(),
            })
        }
    }
    let mut book = FontBook::default();
    let mut declare = |reference: &str| {
        book.declare(
            FontDescriptor {
                family: reference.into(),
                weight: FontRange::single(FontWeight::NORMAL),
                width: FontRange::single(FontWidth::NORMAL),
                style: FontFaceStyle::Normal,
            },
            FontSources {
                sources: vec![FontSource::Url(reference.into())],
                base: Default::default(),
            },
        )
    };
    let face = declare("face.ttf");
    let missing = declare("missing.ttf");
    let clone = book.clone();
    let assets = Assets {
        calls: Cell::new(0),
    };
    assert!(matches!(clone.resolve_cached(face), Err(FontError::UnresolvedFont(id)) if id == face));
    assert_eq!(assets.calls.get(), 0);
    assert_eq!(clone.resolve(face, &assets).unwrap(), Some(FONT));
    assert_eq!(book.resolve(face, &assets).unwrap(), Some(FONT));
    assert_eq!(
        assets.calls.get(),
        1,
        "acquisition through a clone populates the original job cache"
    );
    assert_eq!(
        clone.resolve_cached(face).unwrap().unwrap().as_ptr(),
        book.resolve_cached(face).unwrap().unwrap().as_ptr(),
        "SVG and document consumers share the original allocation"
    );
    assert!(clone.resolve(missing, &assets).unwrap().is_none());
    assert!(book.resolve_cached(missing).unwrap().is_none());
    assert!(book.resolve(missing, &assets).unwrap().is_none());
    assert_eq!(assets.calls.get(), 2, "unavailable results are shared too");
    let selected = book
        .cached_instance(face, FontSelection::default())
        .unwrap()
        .unwrap();
    assert!(selected.face.glyph_index('H').is_some());
}

#[test]
fn bundled_italic_labels_follow_the_post_table_slant_direction() {
    for face in super::super::bundled::faces() {
        let crate::fonts::FontData::Bundled(data) = face.data else {
            unreachable!("bundled faces embed their data")
        };
        let angle = ttf_parser::Face::parse(data, 0).unwrap().italic_angle();
        // OpenType measures italicAngle counter-clockwise, CSS measures slant clockwise.
        let expected = match (face.descriptor.style, -angle) {
            (FontFaceStyle::Left, slant) => slant > 0.0,
            (FontFaceStyle::Right, slant) => slant < 0.0,
            (FontFaceStyle::Italic, slant) => slant == 0.0,
            _ => true,
        };
        assert!(
            expected,
            "{} {:?}: the label must give the italicAngle {angle} direction (CSS Fonts 4 §2.4)",
            face.descriptor.family, face.descriptor.weight
        );
    }
}
