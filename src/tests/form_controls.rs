use super::common;

use crate::images::Image;
use crate::{DisplayItem, LayoutTrace, render_html};
use common::{assert_labelled_pt, local_options, painted_box};

/// A 288pt-wide page area.
const PAGE: &str = "@page{size:360pt 480pt;margin:36pt}body{margin:0}";
const PIXEL: f32 = 0.75;
const PROBE: [u8; 3] = [0x12, 0x34, 0x56];
const OTHER: [u8; 3] = [0x65, 0x43, 0x21];
const ELLIPSIS: &str = "\u{2026}";

fn render(body: &str) -> Result<crate::RenderOutput, crate::Error> {
    render_html(
        &format!(
            "<!doctype html><style>{}{PAGE}</style>{body}",
            common::PROFILE
        ),
        &local_options(),
    )
}

fn trace(body: &str) -> LayoutTrace {
    render(body)
        .unwrap_or_else(|error| panic!("rendering `{body}`: {error}"))
        .layout
        .unwrap()
}

fn texts(trace: &LayoutTrace) -> Vec<String> {
    trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .collect()
}

fn widgets(trace: &LayoutTrace) -> Vec<crate::graphics::Rect> {
    trace
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            DisplayItem::Image(paint) if matches!(paint.image.as_ref(), Image::Svg(_)) => {
                Some(paint.rect)
            }
            _ => None,
        })
        .collect()
}

fn probe_width(body: &str) -> f32 {
    painted_box(&trace(body), PROBE).width
}

#[test]
fn text_fields_measure_size_characters_and_ignore_their_value() {
    let style = "<style>input{background:#123456;border:0;padding:0}</style>";
    let width = |attributes: &str| probe_width(&format!("{style}<input {attributes}>"));
    let [one, eleven, default] = [width("size=1"), width("size=11"), width("")];
    let average = (eleven - one) / 10.0;
    assert!(
        average > 0.0 && one > 0.0,
        "(size-1)*avg + max grows with size: size=1 {one}pt, size=11 {eleven}pt"
    );
    assert_labelled_pt(
        "the default size is 20 characters",
        default,
        one + 19.0 * average,
    );
    assert_labelled_pt(
        "the value does not size a fixed field",
        width("size=11 value='a much longer value than eleven characters'"),
        eleven,
    );
    assert_labelled_pt(
        "size does not apply to number fields, which default to 20 characters",
        width("type=number size=1"),
        default,
    );
    let content = width("value='' style='field-sizing:content'");
    assert!(
        content < one,
        "an empty content-sized field shrinks below one character: {content}pt vs {one}pt"
    );
}

#[test]
fn textareas_measure_cols_characters_and_rows_lines() {
    let style = "<style>textarea{background:#123456;border:0;padding:0;line-height:15pt}\
        input{background:#654321;border:0;padding:0}</style>";
    let rect = |attributes: &str| {
        painted_box(
            &trace(&format!(
                "{style}<textarea {attributes}>text</textarea><input size=1><input size=11>"
            )),
            PROBE,
        )
    };
    let fields = trace(&format!("{style}<input size=1><br><input size=11>"));
    let boxes: Vec<_> = fields
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == OTHER => {
                Some(paint.rect.width)
            }
            _ => None,
        })
        .collect();
    let average = (boxes[1] - boxes[0]) / 10.0;
    let [ten, twenty] = [rect("cols=10"), rect("cols=20")];
    assert_labelled_pt(
        "ten more columns widen the textarea by ten average characters",
        twenty.width - ten.width,
        10.0 * average,
    );
    assert_labelled_pt("the default is 20 columns", rect("").width, twenty.width);
    assert_labelled_pt("two rows by default", rect("").height, 30.0);
    assert_labelled_pt("rows lines high", rect("rows=5").height, 75.0);
}

#[test]
fn drop_down_boxes_fit_their_widest_label_and_show_the_selected_one() {
    let style = "<style>select{background:#123456;border:0;padding:0}</style>";
    let options =
        "<option>Short</option><option selected>Mid</option><option>A much longer label</option>";
    let full = trace(&format!("{style}<select>{options}</select>"));
    let widest = probe_width(&format!(
        "{style}<select><option>A much longer label</option></select>"
    ));
    assert_labelled_pt(
        "a drop-down box is as wide as its widest label",
        painted_box(&full, PROBE).width,
        widest,
    );
    assert_eq!(
        texts(&full)
            .iter()
            .filter(|text| text.as_str() == "Mid")
            .count(),
        1,
        "only the selected label shows: {:?}",
        texts(&full)
    );
    assert_eq!(
        widgets(&full).len(),
        1,
        "the drop-down button is drawn beside the label"
    );
    let none = trace(&format!(
        "{style}<select style='appearance:none'>{options}</select>"
    ));
    assert!(
        widgets(&none).is_empty(),
        "primitive appearance has no drop-down button"
    );
    let list = trace(&format!(
        "<style>select{{background:#123456;line-height:12pt}}</style><select size=3>{options}<option>Hidden</option></select>"
    ));
    assert_labelled_pt(
        "a list box shows its display size of rows",
        painted_box(&list, PROBE).height,
        36.0 + 2.0 * (PIXEL + PIXEL),
    );
}

/// The `disclosure-open` symbol that `select::picker-icon` shows (CSS Counter Styles 3 §6.3).
const DISCLOSURE_OPEN: &str = "\u{25be}";
const CHECKMARK: &str = "\u{2713}";
/// The text an atomic inline stands for on its line.
const OBJECT_REPLACEMENT: &str = "\u{fffc}";

/// The texts of the lines inside form controls, without the lines holding the controls.
fn control_texts(trace: &LayoutTrace) -> Vec<String> {
    texts(trace)
        .into_iter()
        .filter(|text| text != OBJECT_REPLACEMENT)
        .collect()
}

fn line<'t>(trace: &'t LayoutTrace, text: &str) -> &'t crate::layout::TextLine {
    trace
        .pages
        .iter()
        .flatten()
        .find(|line| line.text.trim() == text)
        .unwrap_or_else(|| panic!("no line {text:?} in {:?}", texts(trace)))
}

#[test]
fn base_drop_down_boxes_show_their_button_and_picker_icon_but_not_their_options() {
    // HTML §15.5.16: the select button slot shows the selected option's label, the closed
    // select popover holds the options, and `::picker-icon` follows at the inline end.
    let base = "<style>select{appearance:base-select;background:#123456;width:150pt}</style>";
    let fallback = trace(&format!(
        "{base}<select><option>One</option><option selected>Two</option></select>"
    ));
    assert_eq!(
        control_texts(&fallback),
        ["Two", DISCLOSURE_OPEN],
        "the selected label and the picker icon only"
    );
    assert!(
        widgets(&fallback).is_empty(),
        "base appearance draws no native drop-down button"
    );
    let select = painted_box(&fallback, PROBE);
    let label = line(&fallback, "Two");
    let icon = line(&fallback, DISCLOSURE_OPEN);
    let padding_inline = 0.5 * 12.0;
    assert_labelled_pt(
        "the label starts after the border and 0.5em of padding",
        label.x,
        select.x + PIXEL + padding_inline,
    );
    assert_labelled_pt(
        "`margin-inline-start: auto` pushes the icon to the inline end",
        icon.x + icon.width,
        select.x + select.width - PIXEL - padding_inline,
    );
    let button = trace(&format!(
        "{base}<style>selectedcontent b{{color:#654321}}</style><select><button>\
         <selectedcontent></selectedcontent></button><option>One</option>\
         <option selected><b>Two</b> more</option></select>"
    ));
    assert_eq!(
        control_texts(&button),
        ["Two more", DISCLOSURE_OPEN],
        "the button shows the selectedcontent clone of the selected option"
    );
    let clone = line(&button, "Two more");
    assert_eq!(
        clone.runs[0].color.rgb, OTHER,
        "the cloned `b` element is matched by selectors inside selectedcontent"
    );
    let styled = trace(&format!(
        "{base}<style>select::picker-icon{{content:'v'}}::picker(select){{display:block}}</style>\
         <select><option>One</option></select>"
    ));
    assert_eq!(
        control_texts(&styled),
        ["One", "v"],
        "author `::picker-icon` content; the closed `::picker(select)` stays without a box"
    );
}

#[test]
fn base_list_boxes_render_their_options_with_checkmarks_and_size_rows() {
    let row = 24.0 * PIXEL;
    let list = trace(
        "<style>select{appearance:base-select;background:#123456}optgroup{color:#654321}</style>\
         <select size=2><option>Alpha</option><option selected label=Beta>ignored</option>\
         <optgroup label=Group><option disabled>Gamma</option></optgroup></select>",
    );
    assert_eq!(
        control_texts(&list),
        ["Alpha", CHECKMARK, "Beta", "Group", "Gamma"],
        "options in tree order, the label attribute replacing content, the optgroup label, \
         and only the checked option's checkmark visible"
    );
    assert_labelled_pt(
        "a border-box block size of `size` rows of max(24px, 1lh)",
        painted_box(&list, PROBE).height,
        2.0 * row,
    );
    let beta = line(&list, "Beta");
    let alpha = line(&list, "Alpha");
    let check = line(&list, CHECKMARK);
    assert!(
        check.x + check.width < beta.x,
        "the checkmark precedes its option's content: {check:?} {beta:?}"
    );
    assert_labelled_pt(
        "a hidden checkmark still takes its place in an unchecked option",
        alpha.x,
        beta.x,
    );
    assert_eq!(
        line(&list, "Gamma").color.alpha,
        0.5,
        "a disabled option mixes its colour half with transparent"
    );
    assert_eq!(line(&list, "Group").color.rgb, OTHER);
}

#[test]
fn internal_select_pseudo_classes_select_only_in_the_user_agent_style_sheet() {
    // Selectors 4 §3.9: author and user style sheets know no `:-ferri-*` pseudo-class, so a
    // selector naming one is invalid and its rule is dropped; inside a forgiving list only
    // that selector is.
    let internal = "select:-ferri-base-select option, select:-ferri-list-box option";
    let body = format!(
        "<style>select{{appearance:base-select}}{internal}{{color:#123456}}\
         select:is(:-ferri-list-box, #none) option{{color:#123456}}\
         @supports selector(:-ferri-base-select){{option{{color:#123456}}}}</style>\
         <select size=2><option>Alpha</option><option selected>Beta</option></select>"
    );
    let mut options = local_options();
    options
        .user_stylesheets
        .push(format!("{internal}{{color:#123456}}"));
    let list = render_html(
        &format!(
            "<!doctype html><style>{}{PAGE}</style>{body}",
            common::PROFILE
        ),
        &options,
    )
    .unwrap_or_else(|error| panic!("rendering `{body}`: {error}"))
    .layout
    .unwrap();
    assert_eq!(
        control_texts(&list),
        ["Alpha", CHECKMARK, "Beta"],
        "the user-agent rules for base list boxes still apply"
    );
    for option in ["Alpha", "Beta"] {
        assert_ne!(
            line(&list, option).color.rgb,
            PROBE,
            "no author or user rule naming an internal pseudo-class styles {option}"
        );
    }
}

#[test]
fn checkboxes_and_gauges_are_replaced_by_drawings_only_in_native_appearance() {
    let checkbox = trace("<input type=checkbox checked style='margin:0;font-size:16px'>");
    let rects = widgets(&checkbox);
    assert_eq!(rects.len(), 1, "one native checkbox drawing");
    assert_labelled_pt("13px at a 16px font", rects[0].width, 13.0 * PIXEL);
    let scaled = widgets(&trace(
        "<input type=checkbox style='width:40pt;height:30pt'>",
    ));
    assert_labelled_pt("author sizes scale the widget", scaled[0].width, 40.0);
    assert!(
        widgets(&trace(
            "<input type=checkbox checked style='appearance:none'>"
        ))
        .is_empty(),
        "primitive checkboxes suppress the check mark"
    );
    let meter = widgets(&trace("<meter value=0.5 style='font-size:10pt'></meter>"));
    assert_labelled_pt("a meter is 5em wide", meter[0].width, 50.0);
    assert_labelled_pt("and 1em high", meter[0].height, 10.0);
    let progress = widgets(&trace("<progress style='font-size:10pt'></progress>"));
    assert_labelled_pt("a progress bar is 10em wide", progress[0].width, 100.0);
}

#[test]
fn buttons_and_placeholders_show_their_labels_and_hints() {
    let lines = texts(&trace(
        "<input type=submit><input type=reset><input type=button value=Go>\
         <input placeholder=Hint><input placeholder=Hidden value=Shown>\
         <input type=password value=abc><textarea placeholder='Two&#10;lines'></textarea>",
    ));
    let joined = lines.join("|");
    for expected in [
        "Submit",
        "Reset",
        "Go",
        "Hint",
        "Shown",
        "\u{2022}\u{2022}\u{2022}",
    ] {
        assert!(
            joined.contains(expected),
            "{expected:?} must render: {lines:?}"
        );
    }
    assert!(
        !joined.contains("Hidden") && !joined.contains("abc"),
        "a placeholder hides behind a value and passwords are masked: {lines:?}"
    );
    let placeholder =
        trace("<style>input::placeholder{color:#123456}</style><input placeholder=Hint>");
    let line = placeholder
        .pages
        .iter()
        .flatten()
        .find(|line| line.text.contains("Hint"))
        .unwrap();
    assert_eq!(
        line.runs[0].color.rgb, PROBE,
        "::placeholder styles the hint"
    );
}

#[test]
fn text_field_values_paint_with_the_controls_colour_and_text_paint() {
    let trace = trace(
        "<style>input,textarea{color:#123456;text-shadow:1px 1px;\
         -webkit-text-stroke:1px currentcolor}</style>\
         <input value=Field><input type=date value=2020-01-02><textarea>Area</textarea>",
    );
    for text in ["Field", "2020-01-02", "Area"] {
        let line = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text.contains(text))
            .unwrap_or_else(|| panic!("{text:?} must render: {:?}", texts(&trace)));
        let run = &line.runs[0];
        assert_eq!(
            run.color.rgb, PROBE,
            "{text:?} fills with the control's colour (HTML §15.5): {run:?}"
        );
        assert_eq!(
            run.stroke.map(|stroke| stroke.color.rgb),
            Some(PROBE),
            "{text:?} strokes with currentcolor resolved for the control: {run:?}"
        );
        assert_eq!(
            run.shadows
                .iter()
                .map(|shadow| shadow.color.used.rgb)
                .collect::<Vec<_>>(),
            vec![PROBE],
            "{text:?} casts its currentcolor shadow in the control's colour: {run:?}"
        );
    }
}

#[test]
fn placeholders_and_textareas_ellipse_lines_that_overflow_their_inline_end() {
    // CSS Overflow 3 §3.1: the controls clip, so their lines take `text-overflow`.
    let long = "a value far longer than the control is wide";
    for body in [
        format!("<input size=6 placeholder='{long}'>"),
        format!("<textarea cols=6 style='white-space:pre'>{long}</textarea>"),
    ] {
        let last_run = |css: &str| {
            let trace = trace(&format!("<style>input,textarea{{{css}}}</style>{body}"));
            let line = trace
                .pages
                .iter()
                .flatten()
                .find(|line| line.text.starts_with('a'))
                .unwrap_or_else(|| panic!("{body}: the value renders: {:?}", texts(&trace)));
            line.runs.last().expect("the value has runs").text.clone()
        };
        let clipped = last_run("");
        assert_ne!(
            clipped, ELLIPSIS,
            "{body}: a clipped line ends in its own text"
        );
        assert_eq!(
            last_run("text-overflow:ellipsis"),
            ELLIPSIS,
            "{body}: the overflowing line ends in an ellipsis"
        );
    }
}

#[test]
fn file_selector_buttons_take_their_own_display_float_and_position() {
    // CSS Pseudo 4 §5.1: no restriction on which properties apply to the button.
    let style = "<style>input{width:200pt;border:0;padding:0;font-size:10pt}\
                 ::file-selector-button{border:0;padding:0}";
    let label = |css: &str| {
        let trace = trace(&format!("{style}{css}</style><input type=file>"));
        trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text.contains("Choose File"))
            .map(|line| line.x)
    };
    let start = label("").expect("the button shows its label");
    assert_labelled_pt("an inline button starts the control", start, 36.0);
    assert_eq!(
        label("::file-selector-button{display:none}"),
        None,
        "display:none generates no button"
    );
    for css in [
        "::file-selector-button{float:right}",
        "::file-selector-button{position:absolute;right:0}",
        "input{position:relative}::file-selector-button{position:absolute;right:0}",
    ] {
        let x = label(css).unwrap_or_else(|| panic!("{css}: the button still renders"));
        assert!(
            x > start + 100.0,
            "{css}: the button leaves the line for the control's right edge, at {x}pt"
        );
    }
    let block = label("::file-selector-button{display:block;margin-left:auto}")
        .expect("a block button renders");
    assert!(
        block > start + 100.0,
        "a block-level fit-content button takes auto margins, at {block}pt"
    );
}

#[test]
fn rendered_legends_sit_centred_in_the_block_start_border_with_a_gap() {
    let output = trace(
        "<style>fieldset{margin:0;border:4pt solid #123456;padding:6pt}\
         legend{background:#654321;padding:0;height:20pt;margin:0 0 0 10pt}</style>\
         <fieldset><legend>Legend</legend>Content</fieldset>",
    );
    let fieldset = output
        .display_lists
        .iter()
        .flatten()
        .find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.borders[0].color.rgb == PROBE => {
                Some(paint)
            }
            _ => None,
        })
        .expect("fieldset box");
    let legend = painted_box(&output, OTHER);
    let cut = fieldset
        .legend
        .expect("the fieldset paints around its legend");
    assert_labelled_pt(
        "the legend's border box is centred over the 4pt border",
        fieldset.rect.y + cut.inset + 2.0,
        legend.y + legend.height / 2.0,
    );
    assert_labelled_pt(
        "the fieldset box starts at the legend",
        fieldset.rect.y,
        legend.y,
    );
    assert_labelled_pt(
        "the border gap starts at the legend's border box",
        fieldset.rect.x + cut.gap[0],
        legend.x,
    );
    assert_labelled_pt("and spans it", cut.gap[1] - cut.gap[0], legend.width);
    assert_labelled_pt(
        "the legend is positioned inside the fieldset's inline padding",
        legend.x - fieldset.rect.x,
        4.0 + 6.0 + 10.0,
    );
    let content = output
        .pages
        .iter()
        .flatten()
        .find(|line| line.text.contains("Content"))
        .unwrap();
    assert!(
        content.baseline > legend.y + legend.height + 6.0,
        "the content box follows the 20pt legend area and the block padding"
    );
}

#[test]
fn closed_details_show_only_their_summary_and_hidden_elements_vanish() {
    let lines = texts(&trace(
        "<details><summary>Sum</summary>Body</details><details open>Inner</details>\
         <details>Unseen</details><p hidden>Gone</p><dialog>Closed</dialog>\
         <dialog open>Shown dialog</dialog><div hidden=until-found>Found</div><noscript>Plain</noscript>",
    ));
    assert_eq!(
        lines,
        [
            "\u{25b8} Sum",
            "\u{25be} Details",
            "Inner",
            "\u{25b8} Details",
            "Plain",
            "Shown dialog"
        ],
        "closed contents, closed dialogs and hidden elements do not render"
    );
}

#[test]
fn block_content_alignment_and_self_alignment_offset_their_boxes() {
    let output = trace(
        "<div style='height:100pt;line-height:20pt'>Start</div>\
         <div style='height:100pt;align-content:center;line-height:20pt'>Middle</div>\
         <div style='height:100pt;align-content:end;line-height:20pt'>End</div>\
         <div style='width:288pt'><div style='width:88pt;justify-self:center;background:#123456;height:1pt'></div></div>",
    );
    let baseline = |text: &str| {
        output.pages[0]
            .iter()
            .find(|line| line.text == text)
            .unwrap()
            .baseline
    };
    assert_labelled_pt(
        "a 20pt line centres in a 100pt box",
        baseline("Middle") - baseline("Start"),
        100.0 + 40.0,
    );
    assert_labelled_pt(
        "and ends at its end edge",
        baseline("End") - baseline("Start"),
        200.0 + 80.0,
    );
    assert_labelled_pt(
        "justify-self centres a block in its containing block",
        painted_box(&output, PROBE).x,
        36.0 + 100.0,
    );
}

#[test]
fn framesets_divide_the_viewport_among_their_frames() {
    let output = render_html(
        "<!doctype html><style>@page{size:360pt 480pt;margin:36pt}</style>\
         <frameset cols='72,*' frameborder=0><frame srcdoc='x'><frameset rows='50%,*'><frame><frame></frameset></frameset>",
        &local_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    let mut frames: Vec<_> = output
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            DisplayItem::Image(paint)
                if matches!(
                    paint.image.as_ref(),
                    Image::Viewport(_) | Image::Transparent
                ) =>
            {
                Some([
                    paint.rect.x,
                    paint.rect.y,
                    paint.rect.width,
                    paint.rect.height,
                ])
            }
            _ => None,
        })
        .collect();
    frames.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        frames,
        [
            [36.0, 36.0, 54.0, 408.0],
            [90.0, 36.0, 234.0, 204.0],
            [90.0, 240.0, 234.0, 204.0]
        ],
        "72px absolute, the rest relative; the nested rows split it in half"
    );
}
