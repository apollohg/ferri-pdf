use crate::style::{DEFAULT_PAGE, StyleError, units::Relative};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ContrastPreference {
    #[default]
    NoPreference,
    Less,
    More,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MotionPreference {
    #[default]
    NoPreference,
    Reduce,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PrintPreferences {
    pub color_scheme: ColorScheme,
    pub contrast: ContrastPreference,
    pub reduced_motion: MotionPreference,
}

/// Deterministic print-device settings, independent of author page boxes.
#[derive(Clone, Debug, PartialEq)]
pub struct PrintSettings {
    pub paper: [f32; 2],
    pub preferences: PrintPreferences,
    /// Installed generic and glyph-fallback family preferences.
    pub fonts: crate::fonts::FontPreferences,
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            paper: DEFAULT_PAGE,
            preferences: Default::default(),
            fonts: Default::default(),
        }
    }
}

impl PrintSettings {
    pub fn validate(&self) -> Result<(), StyleError> {
        if self
            .paper
            .iter()
            .any(|side| !side.is_finite() || *side <= 0.0)
        {
            return Err(StyleError::InvalidPrintSettings { paper: self.paper });
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct PrintEnvironment {
    pub paper: [f32; 2],
    pub preferences: PrintPreferences,
    pub relative: Relative<'static>,
    pub supports: crate::style::supports::SupportNames,
    pub custom_media: std::collections::BTreeMap<String, crate::style::conditions::NamedValue>,
}

impl PrintEnvironment {
    pub(crate) fn new(settings: &PrintSettings) -> Result<Self, StyleError> {
        settings.validate()?;
        let initial_font = (!settings.fonts.is_empty()).then(|| {
            std::sync::Arc::new(crate::fonts::InitialFont::from_preferences(&settings.fonts))
        });
        let relative = Relative::media(settings.paper, initial_font);
        Ok(Self {
            paper: settings.paper,
            preferences: settings.preferences.clone(),
            relative,
            custom_media: Default::default(),
            supports: Default::default(),
        })
    }
}
