//! CSS Counter Styles 3: every system against the specification's worked examples, the
//! descriptors that shape a representation, fallback, `extends` and the cascade of rules.
use crate::dom::{DocumentTree, Selector};
use crate::fonts::FontNamespace;
use crate::resources::FileResolver;
use crate::style::{
    ListStyleType, Style, Stylesheet, StylesheetSource,
    counter_style::{CounterStyle, Scope},
};

fn sheet(css: &str) -> Stylesheet {
    Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])
    .unwrap_or_else(|error| panic!("`{css}` must parse: {error}"))
}

fn style(sheet: &Stylesheet, name: &str) -> CounterStyle {
    Scope {
        styles: &sheet.counter_styles,
        namespace: FontNamespace::ROOT,
    }
    .named(name)
}

/// `counter()` representations of `values` in style `name`.
fn representations(css: &str, name: &str, values: &[i128]) -> Vec<String> {
    let sheet = sheet(css);
    let style = style(&sheet, name);
    values
        .iter()
        .map(|value| style.representation(*value).into_text())
        .collect()
}

/// Marker strings, with prefix and suffix, of `values` in style `name`.
fn markers(css: &str, name: &str, values: &[i128]) -> Vec<String> {
    let sheet = sheet(css);
    let style = style(&sheet, name);
    values
        .iter()
        .map(|value| style.marker(*value, false).into_text())
        .collect()
}

fn predefined(name: &str, values: &[i128]) -> Vec<String> {
    values
        .iter()
        .map(|value| {
            CounterStyle::predefined(name)
                .representation(*value)
                .into_text()
        })
        .collect()
}

#[test]
fn cyclic_system_matches_the_triangle_example() {
    // §3.1.1.
    let css = "@counter-style triangle { system: cyclic; symbols: ‣; suffix: \" \"; }";
    assert_eq!(markers(css, "triangle", &[1, 2, 3]), ["‣ ", "‣ ", "‣ "]);
    let css = "@counter-style two { system: cyclic; symbols: a b; }";
    assert_eq!(
        representations(css, "two", &[-1, 0, 1, 2, 3]),
        ["a", "b", "a", "b", "a"],
        "cyclic is defined over all values and uses no negative sign"
    );
}

#[test]
fn fixed_system_matches_the_box_corner_example_and_falls_back() {
    // §3.1.2.
    let css = "@counter-style box-corner { system: fixed; symbols: ◰ ◳ ◲ ◱; suffix: ': '; }";
    assert_eq!(
        markers(css, "box-corner", &[1, 2, 3, 4, 5, 6]),
        ["◰: ", "◳: ", "◲: ", "◱: ", "5: ", "6: "],
        "values past the symbols use the fallback's representation with this style's suffix"
    );
    let css = "@counter-style late { system: fixed 3; symbols: x y; }";
    assert_eq!(
        representations(css, "late", &[2, 3, 4, 5]),
        ["2", "x", "y", "5"]
    );
}

#[test]
fn symbolic_system_matches_the_footnote_and_legal_examples() {
    // §3.1.3.
    let css = "@counter-style footnote { system: symbolic; symbols: '*' ⁑ † ‡; suffix: \" \"; }";
    assert_eq!(
        representations(css, "footnote", &[1, 2, 3, 4, 5, 6]),
        ["*", "⁑", "†", "‡", "**", "⁑⁑"]
    );
    let css = "@counter-style upper-alpha-legal { system: symbolic; symbols: A B C D E F G H I J K L M N O P Q R S T U V W X Y Z; }";
    assert_eq!(
        representations(css, "upper-alpha-legal", &[1, 26, 27, 28, 53]),
        ["A", "Z", "AA", "BB", "AAA"]
    );
    assert_eq!(
        predefined("upper-alpha", &[27, 28, 53]),
        ["AA", "AB", "BA"],
        "the alphabetic upper-alpha diverges from the symbolic style after 26"
    );
}

#[test]
fn symbolic_representations_past_sixty_code_points_fall_back() {
    // §2: the size of a symbolic representation is linear in the value.
    let css = "@counter-style star { system: symbolic; symbols: '*'; }";
    let values = representations(css, "star", &[60, 61]);
    assert_eq!(values[0], "*".repeat(60));
    assert_eq!(values[1], "61");
}

#[test]
fn alphabetic_system_numbers_bijectively() {
    // §3.1.4.
    let css = "@counter-style go { system: alphabetic; symbols: '○' '●'; suffix: \" \"; }";
    assert_eq!(
        representations(css, "go", &[1, 2, 3, 4, 5, 6, 7]),
        ["○", "●", "○○", "○●", "●○", "●●", "○○○"]
    );
    assert_eq!(
        representations(css, "go", &[0, -1]),
        ["0", "-1"],
        "alphabetic systems are defined only over positive values"
    );
}

#[test]
fn numeric_system_matches_the_trinary_example() {
    // §3.1.5.
    let css = "@counter-style trinary { system: numeric; symbols: '0' '1' '2'; }";
    assert_eq!(
        markers(css, "trinary", &[1, 2, 3, 4, 5, 6, 0, -3]),
        ["1. ", "2. ", "10. ", "11. ", "12. ", "20. ", "0. ", "-10. "]
    );
}

#[test]
fn additive_system_matches_the_dice_example_and_its_failures() {
    // §3.1.6.
    let css = "@counter-style dice { system: additive; additive-symbols: 6 ⚅, 5 ⚄, 4 ⚃, 3 ⚂, 2 ⚁, 1 ⚀; suffix: \" \"; }";
    assert_eq!(
        representations(css, "dice", &[1, 2, 3, 11, 12, 13]),
        ["⚀", "⚁", "⚂", "⚅⚄", "⚅⚅", "⚅⚅⚀"]
    );
    assert_eq!(
        representations(css, "dice", &[0]),
        ["0"],
        "zero needs a zero-weight tuple"
    );
    let css = "@counter-style gap { system: additive; additive-symbols: 3 a, 2 b; }";
    assert_eq!(
        representations(css, "gap", &[3, 4, 5]),
        ["a", "4", "ab"],
        "the greedy algorithm cannot represent 4 and falls back"
    );
    let css = "@counter-style unordered { system: additive; additive-symbols: 1 a, 2 b; }";
    assert_eq!(
        representations(css, "unordered", &[2]),
        ["2"],
        "weights out of descending order invalidate the descriptor, so the rule defines nothing"
    );
}

#[test]
fn extends_reuses_the_algorithm_and_unspecified_descriptors() {
    // §3.1.7.
    let css = "@counter-style decimal-paren { system: extends decimal; suffix: \") \"; }
        @counter-style decimal-bracket { system: extends decimal-paren; prefix: '['; }
        @counter-style roman-paren { system: extends upper-roman; negative: '(' ')'; }";
    assert_eq!(
        markers(css, "decimal-paren", &[1, 2, 3]),
        ["1) ", "2) ", "3) "]
    );
    assert_eq!(
        markers(css, "decimal-bracket", &[12]),
        ["[12) "],
        "the suffix comes through the chain"
    );
    assert_eq!(
        representations(css, "roman-paren", &[4, 3999, 4000]),
        ["IV", "MMMCMXCIX", "4000"],
        "the extended style's range comes with its algorithm"
    );
    let css = "@counter-style spelled { system: extends unknown-style; suffix: '/'; }";
    assert_eq!(
        markers(css, "spelled", &[7]),
        ["7/"],
        "an unknown extended style is decimal"
    );
    let css = "@counter-style with-symbols { system: extends decimal; symbols: a b; }";
    assert_eq!(
        markers(css, "with-symbols", &[1]),
        ["1. "],
        "extends with symbols is invalid and defines no style"
    );
}

#[test]
fn extends_cycles_extend_decimal_for_every_member() {
    // §3.1.7.
    let css = "@counter-style a { system: extends b; suffix: 'A'; }
        @counter-style b { system: extends a; prefix: 'B'; }
        @counter-style c { system: extends a; }
        @counter-style self { system: extends self; suffix: '!'; }";
    assert_eq!(markers(css, "a", &[3]), ["3A"]);
    assert_eq!(
        markers(css, "b", &[3]),
        ["B3. "],
        "a member of the cycle takes decimal's descriptors, not another member's"
    );
    assert_eq!(
        markers(css, "c", &[3]),
        ["3A"],
        "a style leading into the cycle still extends its member"
    );
    assert_eq!(markers(css, "self", &[5]), ["5!"]);
}

#[test]
fn negative_and_pad_wrap_the_representation() {
    // §3.2 and §3.6.
    let css = "@counter-style money { system: extends decimal; negative: '(' ')'; }
        @counter-style padded { system: extends decimal; pad: 3 '0'; }";
    assert_eq!(
        representations(css, "money", &[-2, -1, 0, 1]),
        ["(2)", "(1)", "0", "1"]
    );
    assert_eq!(
        representations(css, "padded", &[1, 20, 300, 4000, -5]),
        ["001", "020", "300", "4000", "-05"]
    );
    let css = "@counter-style wide { system: extends decimal; pad: 61 '0'; }";
    assert_eq!(
        representations(css, "wide", &[1]),
        ["1"],
        "padding past sixty code points falls back"
    );
}

#[test]
fn ranges_select_the_fallback_through_loops() {
    // §3.5 and §3.7.
    let css = "@counter-style low { system: extends decimal; range: 1 3; fallback: lower-roman; }
        @counter-style odd { system: cyclic; symbols: odd; range: 1 1, 3 3, 5 5; fallback: even; }
        @counter-style even { system: cyclic; symbols: even; range: 2 2, 4 4; fallback: odd; }
        @counter-style lost { system: cyclic; symbols: x; range: 1 1; fallback: nowhere; }
        @counter-style inverted { system: cyclic; symbols: x; range: 5 1; }
        @counter-style unbounded { system: extends lower-roman; range: auto; }";
    assert_eq!(representations(css, "low", &[3, 4]), ["3", "iv"]);
    assert_eq!(
        representations(css, "odd", &[1, 2, 3, 4, 6]),
        ["odd", "even", "odd", "even", "6"],
        "mutual fallbacks are no loop until neither style represents the value"
    );
    assert_eq!(representations(css, "lost", &[2]), ["2"]);
    assert_eq!(
        representations(css, "inverted", &[7]),
        ["x"],
        "a lower bound above its upper bound invalidates the descriptor"
    );
    assert_eq!(
        representations(css, "unbounded", &[0, 4000]),
        ["0", "mmmm"],
        "`auto` is the extended additive system's range"
    );
}

#[test]
fn rules_cascade_atomically_by_name() {
    // §3: the last rule of a name replaces earlier ones entirely.
    let css = "@counter-style x { system: cyclic; symbols: a; suffix: ')'; }
        @counter-style x { system: cyclic; symbols: b; }
        @counter-style y { system: cyclic; symbols: c; }
        @counter-style y { system: numeric; symbols: d; }";
    assert_eq!(markers(css, "x", &[1]), ["b. "]);
    assert_eq!(
        markers(css, "y", &[1]),
        ["c. "],
        "a later rule that defines no style does not replace an earlier one"
    );
    let css = "@layer base { @counter-style z { system: cyclic; symbols: layered; } }
        @counter-style z { system: cyclic; symbols: unlayered; }
        @layer base { @counter-style z { system: cyclic; symbols: later; } }";
    assert_eq!(
        representations(css, "z", &[1]),
        ["unlayered"],
        "unlayered rules outrank layered ones whatever their order"
    );
}

#[test]
fn names_are_case_sensitive_except_predefined_ones() {
    // §3.
    let css = "@counter-style Thumbs { system: cyclic; symbols: up; }
        @counter-style LOWER-ROMAN { system: cyclic; symbols: replaced; }";
    assert_eq!(representations(css, "Thumbs", &[1]), ["up"]);
    assert_eq!(
        representations(css, "thumbs", &[1]),
        ["1"],
        "a differently cased custom name is unknown, so decimal"
    );
    assert_eq!(
        representations(css, "Lower-Roman", &[1]),
        ["replaced"],
        "predefined names lowercase in rules and references, so the rule overrides lower-roman"
    );
    let css = "@counter-style decimal { system: cyclic; symbols: no; }
        @counter-style DISC { system: cyclic; symbols: no; }
        @counter-style none { system: cyclic; symbols: no; }";
    let sheet = sheet(css);
    assert_eq!(style(&sheet, "decimal").representation(1).into_text(), "1");
    assert_eq!(style(&sheet, "disc").representation(1).into_text(), "•");
    assert_eq!(
        style(&sheet, "none").representation(1).into_text(),
        "",
        "`none` represents nothing and names no rule"
    );
}

#[test]
fn predefined_numeric_and_alphabetic_styles() {
    // §6.1 and §6.2.
    assert_eq!(
        predefined("decimal-leading-zero", &[1, 10, -5]),
        ["01", "10", "-5"]
    );
    assert_eq!(predefined("arabic-indic", &[100]), ["١٠٠"]);
    assert_eq!(predefined("cjk-decimal", &[101]), ["一〇一"]);
    assert_eq!(
        predefined("lower-roman", &[1, 4, 3999, 4000]),
        ["i", "iv", "mmmcmxcix", "4000"]
    );
    assert_eq!(
        predefined("hebrew", &[15, 16, 99, 100]),
        ["טו", "טז", "צט", "ק"]
    );
    assert_eq!(predefined("armenian", &[1, 100, 3]), ["Ա", "Ճ", "Գ"]);
    assert_eq!(predefined("georgian", &[99, 100]), ["ჟთ", "რ"]);
    assert_eq!(predefined("lower-greek", &[24, 25]), ["ω", "αα"]);
    assert_eq!(
        predefined("katakana-iroha", &[1, 47, 48]),
        ["イ", "ス", "イイ"]
    );
    assert_eq!(
        predefined("cjk-earthly-branch", &[1, 12, 13]),
        ["子", "亥", "13"]
    );
    let marker = |name: &str| CounterStyle::predefined(name).marker(1, false).into_text();
    assert_eq!(marker("hiragana"), "あ、");
    assert_eq!(marker("disc"), "• ");
    assert_eq!(marker("upper-latin"), "A. ");
}

#[test]
fn predefined_disclosure_styles_follow_the_inline_direction() {
    // §6.3.
    let open = CounterStyle::predefined("disclosure-open");
    let closed = CounterStyle::predefined("disclosure-closed");
    assert_eq!(open.marker(1, false).into_text(), "▾ ");
    assert_eq!(closed.marker(1, false).into_text(), "▸ ");
    assert_eq!(closed.marker(1, true).into_text(), "◂ ");
}

#[test]
fn longhand_east_asian_styles_match_the_specification_table() {
    // §7.1: the table of 0, 1, 2, 3, 10, 11, 99, 100, 101 and 6001.
    let values = [0, 1, 2, 3, 10, 11, 99, 100, 101, 6001];
    for (name, expected) in [
        (
            "japanese-informal",
            [
                "〇",
                "一",
                "二",
                "三",
                "十",
                "十一",
                "九十九",
                "百",
                "百一",
                "六千一",
            ],
        ),
        (
            "japanese-formal",
            [
                "零",
                "壱",
                "弐",
                "参",
                "壱拾",
                "壱拾壱",
                "九拾九",
                "壱百",
                "壱百壱",
                "六阡壱",
            ],
        ),
        (
            "korean-hangul-formal",
            [
                "영",
                "일",
                "이",
                "삼",
                "일십",
                "일십일",
                "구십구",
                "일백",
                "일백일",
                "육천일",
            ],
        ),
        (
            "korean-hanja-informal",
            [
                "零",
                "一",
                "二",
                "三",
                "十",
                "十一",
                "九十九",
                "百",
                "百一",
                "六千一",
            ],
        ),
        (
            "korean-hanja-formal",
            [
                "零",
                "壹",
                "貳",
                "參",
                "壹拾",
                "壹拾壹",
                "九拾九",
                "壹百",
                "壹百壹",
                "六仟壹",
            ],
        ),
        (
            "simp-chinese-informal",
            [
                "零",
                "一",
                "二",
                "三",
                "十",
                "十一",
                "九十九",
                "一百",
                "一百零一",
                "六千零一",
            ],
        ),
        (
            "simp-chinese-formal",
            [
                "零",
                "壹",
                "贰",
                "叁",
                "壹拾",
                "壹拾壹",
                "玖拾玖",
                "壹佰",
                "壹佰零壹",
                "陆仟零壹",
            ],
        ),
        (
            "trad-chinese-informal",
            [
                "零",
                "一",
                "二",
                "三",
                "十",
                "十一",
                "九十九",
                "一百",
                "一百零一",
                "六千零一",
            ],
        ),
        (
            "trad-chinese-formal",
            [
                "零",
                "壹",
                "貳",
                "參",
                "壹拾",
                "壹拾壹",
                "玖拾玖",
                "壹佰",
                "壹佰零壹",
                "陸仟零壹",
            ],
        ),
    ] {
        assert_eq!(predefined(name, &values), expected, "{name}");
    }
    assert_eq!(
        predefined("simp-chinese-informal", &[110, 120, 1010, -15]),
        ["一百一十", "一百二十", "一千零一十", "负十五"]
    );
    assert_eq!(
        predefined("cjk-ideographic", &[11, -1]),
        predefined("trad-chinese-informal", &[11, -1])
    );
    assert_eq!(
        predefined("japanese-informal", &[-5, 10000]),
        ["マイナス五", "一〇〇〇〇"],
        "past 9999 the fallback is cjk-decimal"
    );
    assert_eq!(
        CounterStyle::predefined("korean-hangul-formal")
            .marker(-1, false)
            .into_text(),
        "마이너스 일, "
    );
    let css = "@counter-style extended { system: extends simp-chinese-formal; range: auto; }";
    assert_eq!(
        representations(css, "extended", &[9999, 10000]),
        ["玖仟玖佰玖拾玖", "一〇〇〇〇"],
        "`auto` for an extended complex style is that style's range"
    );
}

#[test]
fn ethiopic_numeric_matches_the_specification_examples() {
    // §7.2.
    assert_eq!(
        predefined("ethiopic-numeric", &[1, 100, 78_010_092, 780_100_000_092]),
        ["፩", "፻", "፸፰፻፩፼፺፪", "፸፰፻፩፼፼፺፪"]
    );
    assert_eq!(
        predefined("ethiopic-numeric", &[0, 10, 11]),
        ["0", "፲", "፲፩"]
    );
    assert_eq!(
        CounterStyle::predefined("ethiopic-numeric")
            .marker(2, false)
            .into_text(),
        "፪/ "
    );
}

fn list_style_type(css: &str) -> Result<ListStyleType, crate::style::StyleError> {
    let sheet = sheet(css);
    let html = DocumentTree::parse("<ol><li id=item>x</li></ol>").unwrap();
    let item = html
        .select(&Selector::parse("#item").unwrap())
        .next()
        .unwrap();
    let ol = html.select(&Selector::parse("ol").unwrap()).next().unwrap();
    let parent = sheet.computed_with_resolver(ol, &Style::default(), &FileResolver::default())?;
    sheet
        .computed_with_resolver(item, &parent, &FileResolver::default())
        .map(|style| style.list_style_type)
}

#[test]
fn symbols_function_defines_anonymous_styles() {
    // §4.
    let marker = |css: &str, values: &[i128]| {
        let ListStyleType::Counter(style) = list_style_type(css).unwrap() else {
            panic!("`{css}` must compute to a counter style");
        };
        values
            .iter()
            .map(|value| style.marker(*value, false).into_text())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        marker(
            "li { list-style-type: symbols(\"*\" \"\\2020\" \"\\2021\" \"\\A7\") }",
            &[1, 4, 5, 7]
        ),
        ["* ", "§ ", "** ", "‡‡ "]
    );
    assert_eq!(
        marker(
            "li { list-style-type: symbols(cyclic \"*\" \"\\2020\" \"\\2021\" \"\\A7\") }",
            &[5, 7]
        ),
        ["* ", "‡ "]
    );
    assert_eq!(
        marker("li { list-style-type: symbols(fixed 'a' 'b') }", &[2, 3]),
        ["b ", "3 "]
    );
    assert_eq!(
        list_style_type(
            "ol { list-style-type: square } li { list-style-type: symbols(alphabetic 'a') }"
        )
        .unwrap(),
        ListStyleType::Counter(CounterStyle::predefined("square")),
        "an alphabetic symbols() with one symbol is invalid, so the declaration is dropped"
    );
    assert_eq!(
        list_style_type("li { list-style-type: '→ ' }").unwrap(),
        ListStyleType::String("→ ".into())
    );
    assert!(
        matches!(
            list_style_type("li { list-style-type: symbols(url(a.png)) }"),
            Ok(ListStyleType::Counter(_))
        ),
        "image symbols compute"
    );
}

#[test]
fn image_symbols_stand_at_their_places_in_representations() {
    // §3.8: image symbols are symbols like strings, and count as one grapheme for `pad`.
    let css = "@counter-style bits { system: numeric; symbols: '0' url(one.svg); \
               prefix: url(start.svg); suffix: '.'; pad: 4 '0'; }";
    let sheet = sheet(css);
    let style = style(&sheet, "bits");
    let image = |representation: &crate::style::Representation, index: usize| {
        let (offset, image) = &representation.images()[index];
        let crate::paint::BackgroundImage::Url { url, .. } = image.as_ref() else {
            panic!("image symbol {index} is a URL image: {image:?}");
        };
        (
            *offset,
            url.rsplit('/').next().unwrap_or_default().to_owned(),
        )
    };
    let representation = style.representation(6);
    assert_eq!(
        representation.text(),
        "00",
        "6 is one-one-zero, padded to four symbols: {representation:?}"
    );
    assert_eq!(
        (0..2)
            .map(|index| image(&representation, index))
            .collect::<Vec<_>>(),
        [(1, "one.svg".to_owned()), (1, "one.svg".to_owned())],
        "the ones follow the single padding zero: {representation:?}"
    );
    let marker = style.marker(1, false);
    assert_eq!(marker.text(), "000.", "{marker:?}");
    assert_eq!(
        (0..2)
            .map(|index| image(&marker, index))
            .collect::<Vec<_>>(),
        [(0, "start.svg".to_owned()), (3, "one.svg".to_owned())],
        "the prefix image precedes the padding and the representation: {marker:?}"
    );
}
