# Local changes to html5ever 0.39.0

Upstream identity: `{'git': {'sha1': 'ce64836c685025a5fef0860fa2e9c80b2683e8d0'}, 'path_in_vcs': 'html5ever'}`. Original crates.io archive SHA-256: `46a1761807faccc9a19e86944bbf40610014066306f96edcdedc2fb714bcb7b8`. Original source headers and Apache/MIT licence files are retained.

The selected HTML snapshot defines `shadowrootmode` as an enumerated attribute with ASCII-case-insensitive keyword matching. The parser's declarative-shadow decision compared its value using case-sensitive string equality, bypassing the TreeSink hook for `OpEn` and `CLOSED`. Use the existing case-insensitive string comparison before calling the same hook; preserve original attribute values and all native tree-construction/fallback algorithms. The public `shadow_root_host_validation_and_ascii_mode_matching` regression observed `Light` instead of `Shadow` for `OpEn` before this correction.
