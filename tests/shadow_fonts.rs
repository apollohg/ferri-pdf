mod common;
use common::{PROFILE, assert_pt, render_trace};
use ferri_pdf::{FileResolver, Resource, ResourceBase, ResourceError, ResourceResolver};
use std::cell::Cell;

const OUTER: &str = "@font-face{font-family:Shared;src:local('DejaVu Serif')}body{font-family:Shared;--family:Shared}";

#[test]
fn shadow_font_references_shape_with_the_correct_faces() {
    let trace = render_trace(&format!(
        "<style>{PROFILE}{OUTER}</style><p>Outer WWWWiiii</p><div><template shadowrootmode=open><style>@font-face{{font-family:Shared;src:local('DejaVu Sans')}}.fresh{{font-family:Shared}}</style><p>Inherited WWWWiiii</p><p class=fresh>Fresh WWWWiiii</p></template></div><div><template shadowrootmode=closed><style>@font-face{{font-family:Shared;src:local('DejaVu Sans Mono')}}p{{font-family:Shared}}</style><p>Sibling WWWWiiii</p></template></div>"
    ));
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    let expected = [
        ("Outer WWWWiiii", "DejaVu Serif"),
        ("Inherited WWWWiiii", "DejaVu Serif"),
        ("Fresh WWWWiiii", "DejaVu Sans"),
        ("Sibling WWWWiiii", "DejaVu Sans Mono"),
    ];
    assert_eq!(lines.len(), expected.len());
    for (line, (text, family)) in lines.iter().zip(expected) {
        assert_eq!(line.text, text);
        let reference = render_trace(&format!(
            "<style>{PROFILE}body{{font-family:'{family}'}}</style><p>{text}</p>"
        ));
        assert_pt(line.width, reference.pages[0][0].width);
    }
}

#[test]
fn inaccessible_shadow_font_sources_are_not_loaded_but_selected_denials_propagate() {
    const DENIED_FONT: &str = "https://denied.example/font.ttf";
    #[derive(Default)]
    struct Policy {
        denied: Cell<usize>,
        files: FileResolver,
    }
    impl ResourceResolver for Policy {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            let url = base.resolve(reference)?;
            if url.as_str() == DENIED_FONT {
                self.denied.set(self.denied.get() + 1);
                return Err(ResourceError::Denied(url));
            }
            self.files.resolve(reference, base)
        }
    }
    for selected in [false, true] {
        let policy = Policy::default();
        let declaration = if selected {
            "style='font-family:Shared'"
        } else {
            ""
        };
        let html = format!(
            "<style>{OUTER}</style><p>Outside</p><div><template shadowrootmode=closed><style>@font-face{{font-family:Shared;src:url({DENIED_FONT})}}</style><p {declaration}>Inside</p></template></div>"
        );
        let result = ferri_pdf::render_with_resolver(&html, &common::local_options(), &policy);
        if selected {
            assert!(
                matches!(
                    &result,
                    Err(ferri_pdf::Error::Css(ferri_pdf::StyleError::Resource(
                        ResourceError::Denied(_)
                    )))
                ),
                "selected shadow font policy denial must retain its typed error: {:?}",
                result.as_ref().err()
            );
            assert_eq!(policy.denied.get(), 1);
        } else {
            assert!(
                result.is_ok(),
                "an inherited document font must not acquire an identically named private shadow font"
            );
            assert_eq!(policy.denied.get(), 0);
        }
    }
}
