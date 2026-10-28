//! Pinned inputs shared by the grammar compiler and language consumers.

pub fn css_directory() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("css")
}

pub mod languages {
    mod generated {
        include!("languages/aliases.rs");
    }
    pub const LANGUAGE: &[(&str, &str)] = generated::LANGUAGE;
    pub const SCRIPT: &[(&str, &str)] = generated::SCRIPT;
    pub const REGION: &[(&str, &str)] = generated::REGION;
    pub const VARIANT: &[(&str, &str)] = generated::VARIANT;
    pub const TAG: &[(&str, &str)] = generated::TAG;
    pub const EXTLANG: &[(&str, &str)] = generated::EXTLANG;
}

pub mod quotes {
    mod generated {
        include!("cldr/quotes.rs");
    }
    pub const SYSTEMS: &[[[&str; 2]; 2]] = generated::SYSTEMS;
    pub const LOCALES: &[(&str, usize)] = generated::LOCALES;
}
