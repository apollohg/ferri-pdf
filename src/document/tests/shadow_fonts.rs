use crate::document::{BlockContent, StyledDocument};
use ferri_pdf_resources::{FileResolver, ResourceBase};

const SERIF: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSerif.ttf");
const SANS: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSans.ttf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSansMono.ttf");
const INITIAL: &[u8] = include_bytes!("../../../assets/fonts/LiberationSerif-Regular.ttf");
const OUTER: &str = "@font-face{font-family:Shared;src:local('DejaVu Serif')}body{font-family:Shared;--family:Shared}";
const INNER: &str = "@font-face{font-family:Shared;src:local('DejaVu Sans')}";

fn assert_faces(html: &str, expected: &[(&str, &[u8])]) {
    let resolver = FileResolver::default();
    let document = StyledDocument::parse(html, &ResourceBase::default(), &[], &resolver)
        .expect("the shadow font document must format");
    let mut pending = vec![document.root()];
    let mut found = vec![false; expected.len()];
    while let Some(block) = pending.pop() {
        for child in block.children() {
            match child {
                BlockContent::Events(_) => {}
                BlockContent::Block(child) | BlockContent::Positioned(child) => pending.push(child),
                BlockContent::Inline(paragraph) => {
                    for span in paragraph.spans() {
                        let text = &paragraph.text()[span.range.clone()];
                        for (index, (label, data)) in expected.iter().enumerate() {
                            if text == *label {
                                let id = span.style.font_candidates()[0];
                                let bytes = document
                                    .styles()
                                    .fonts
                                    .resolve(id, &resolver)
                                    .unwrap()
                                    .unwrap();
                                assert!(
                                    bytes == *data,
                                    "{label:?} must use the exact expected font data; selected {:?}",
                                    document.styles().fonts.face(id).unwrap().descriptor()
                                );
                                found[index] = true;
                            }
                        }
                    }
                    pending.extend(paragraph.atomics().iter().map(|anchor| &anchor.block));
                }
            }
        }
    }
    for (index, (label, _)) in expected.iter().enumerate() {
        assert!(
            found[index],
            "the real formatting tree must contain {label:?}"
        );
    }
}

#[test]
fn font_names_stay_in_their_tree_and_inherited_references_keep_their_origin() {
    assert_faces(
        &format!(
            "<style>{OUTER}</style><p>Outer</p><div><template shadowrootmode=open><style>{INNER}.local{{font-family:Shared}}</style><p>Inherited</p><p class=local>Fresh</p><div><template shadowrootmode=closed><p style='font-family:Shared'>Through sheetless root</p></template></div></template></div><div><template shadowrootmode=closed><style>@font-face{{font-family:Shared;src:local('DejaVu Sans Mono')}}p{{font-family:Shared}}</style><p>Sibling</p></template></div><p>After</p>"
        ),
        &[
            ("Outer", SERIF),
            ("Inherited", SERIF),
            ("Fresh", SANS),
            ("Through sheetless root", SANS),
            ("Sibling", MONO),
            ("After", SERIF),
        ],
    );
}

#[test]
fn host_part_slot_inline_and_variable_references_use_the_declaration_tree() {
    assert_faces(
        &format!(
            "<style>{OUTER}#host::part(outer){{font-family:Shared}}#host::part(variable){{font-family:var(--inner-family)}}</style><div id=host><template shadowrootmode=open><style>{INNER}:host{{font-family:Shared;--inner-family:Shared}}::slotted(.fresh){{font-family:Shared}}::slotted(.variable){{font-family:var(--family)}}#local::before{{content:'Generated';font-family:Shared}}.reset{{font-family:initial}}.inherit{{font-family:inherit}}.invalid{{font-family:var(--missing)}}</style><p>Host inherited</p><p part=outer>Outer part</p><p part=variable>Variable part</p><p id=local></p><p class=reset>Initial</p><p class=inherit>Explicit inherit</p><p class=invalid>Invalid unset</p><p style='font-family:Shared'>Inline shadow</p><slot></slot></template><p class=fresh>Fresh slot</p><p class=variable>Variable slot</p><p style='font-family:Shared'>Inline light</p><font face=Shared>Light hint</font></div>"
        ),
        &[
            ("Host inherited", SANS),
            ("Outer part", SERIF),
            ("Variable part", SERIF),
            ("Generated", SANS),
            ("Initial", INITIAL),
            ("Explicit inherit", SANS),
            ("Invalid unset", SANS),
            ("Inline shadow", SANS),
            ("Fresh slot", SANS),
            ("Variable slot", SANS),
            ("Inline light", SERIF),
            ("Light hint", SERIF),
        ],
    );
}

#[test]
fn rollback_restores_the_selected_font_reference_context() {
    assert_faces(
        &format!(
            "<style>{OUTER}#host{{font-family:Shared!important}}</style><div id=host><template shadowrootmode=open><style>{INNER}:host{{font-family:monospace!important;font-family:revert-layer!important}}</style><p>Restored outer</p></template></div><div><template shadowrootmode=open><style>{INNER}@layer base,over;@layer base{{p{{font-family:Shared}}}}@layer over{{p{{font-family:monospace;font-family:revert-layer}}}}</style><p>Restored inner</p></template></div>"
        ),
        &[("Restored outer", SERIF), ("Restored inner", SANS)],
    );
}

#[test]
fn nearest_family_masks_an_ancestor_with_a_better_matching_weight() {
    const BOLD: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSans-Bold.ttf");
    assert_faces(
        &format!(
            "<style>{OUTER}</style><div><template shadowrootmode=closed><style>@font-face{{font-family:Shared;src:local('DejaVu Sans Bold');font-weight:700}}p{{font-family:Shared;font-weight:400}}</style><p>Nearest family</p></template></div>"
        ),
        &[("Nearest family", BOLD)],
    );
}

#[test]
fn projected_shadow_hosts_keep_name_lookup_separate_from_composed_inheritance() {
    assert_faces(
        &format!(
            "<style>{OUTER}</style><div><template shadowrootmode=open><style>{INNER}slot{{font-family:Shared}}</style><slot></slot></template><article><template shadowrootmode=closed><p>Composed inheritance</p><p style='font-family:Shared'>Physical name lookup</p></template></article></div>"
        ),
        &[
            ("Composed inheritance", SANS),
            ("Physical name lookup", SERIF),
        ],
    );
}
