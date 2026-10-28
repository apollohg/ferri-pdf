# Local changes to cssparser 0.37.0

Upstream: [servo/rust-cssparser](https://github.com/servo/rust-cssparser/tree/4c49486494fb24dc01390e3baca9698ef1744c71), revision `4c49486494fb24dc01390e3baca9698ef1744c71`. The crates.io archive SHA-256 is `8c9cdaae01d5ed7882b04d795e7f752f46ff52d2fa3b50a20d28c464510bba98`. The upstream licence, headers and source metadata are retained.

Add `Parser::expect_no_error_token_with_limit` with a typed `ComponentParseError`, separating ordinary syntax errors from exhaustion of an explicit component budget. It and the existing validator use one structural implementation; no alternative tokenizer or component grammar is added. The existing entry point, return type and `BasicParseErrorKind` variants remain compatible with existing consumers. The style crate imports the shared named budget, retaining its established value and existing typed diagnostics.

The SVG parser preserves depth exhaustion across ordinary CSS error recovery and reports a typed SVG source error before conversion. Real image-pipeline regressions accept 255 nested component functions in inline and stylesheet custom-property values, retain the neighboring display declaration's geometry, and reject 256 with the correct limit. Existing common style/selector boundary tests cover their original paths. No larger test-thread stack or lower source limit is used.
