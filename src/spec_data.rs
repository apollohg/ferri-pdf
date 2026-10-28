//! Pinned language and quotation tables generated from the data in `spec/`.

pub(crate) mod languages {
    mod generated {
        include!("../spec/languages/aliases.rs");
    }
    pub(crate) const LANGUAGE: &[(&str, &str)] = generated::LANGUAGE;
    pub(crate) const SCRIPT: &[(&str, &str)] = generated::SCRIPT;
    pub(crate) const REGION: &[(&str, &str)] = generated::REGION;
    pub(crate) const VARIANT: &[(&str, &str)] = generated::VARIANT;
    pub(crate) const TAG: &[(&str, &str)] = generated::TAG;
    pub(crate) const EXTLANG: &[(&str, &str)] = generated::EXTLANG;
}

pub(crate) mod quotes {
    mod generated {
        include!("../spec/cldr/quotes.rs");
    }
    pub(crate) const SYSTEMS: &[[[&str; 2]; 2]] = generated::SYSTEMS;
    pub(crate) const LOCALES: &[(&str, usize)] = generated::LOCALES;
}
