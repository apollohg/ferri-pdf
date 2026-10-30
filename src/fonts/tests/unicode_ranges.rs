use crate::fonts::UnicodeRanges;

#[test]
fn ranges_merge_overlapping_and_adjacent_intervals_in_code_point_order() {
    let ranges = UnicodeRanges::new([
        (0x400, 0x4FF),
        (0x20, 0x7F),
        (0x80, 0xFF),
        (0x450, 0x52F),
        (0x10_FFFF, 0x10_FFFF),
        (0x30, 0x39),
    ]);
    assert_eq!(
        ranges.intervals(),
        [(0x20, 0xFF), (0x400, 0x52F), (0x10_FFFF, 0x10_FFFF)],
        "unsorted, overlapping and touching intervals collapse into disjoint sorted ones"
    );
    for (ch, expected) in [
        ('\u{1F}', false),
        (' ', true),
        ('\u{FF}', true),
        ('\u{100}', false),
        ('\u{3FF}', false),
        ('\u{400}', true),
        ('\u{52F}', true),
        ('\u{530}', false),
        ('\u{10FFFE}', false),
        ('\u{10FFFF}', true),
    ] {
        assert_eq!(
            ranges.contains(ch),
            expected,
            "U+{:04X} lies {} the merged ranges {:X?}",
            u32::from(ch),
            if expected { "inside" } else { "outside" },
            ranges.intervals()
        );
    }
}
