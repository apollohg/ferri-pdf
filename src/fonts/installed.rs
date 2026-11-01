//! Fonts installed on the system or in configured directories. `fontdb` finds the files; a
//! scan keeps only face metadata, and a face's file is read when rendering first uses it.
use crate::fonts::loading::{ITALIC_AXIS, SLANT_AXIS, WEIGHT_AXIS, WIDTH_AXIS};
use crate::fonts::{
    FaceSettings, FontData, FontDescriptor, FontFace, FontFaceStyle, FontNamespace,
    FontPreferences, FontRange, FontSlant, FontWeight, FontWidth, matching, sfnt,
};
use crate::work::{self, Work};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

const NAME_TABLE: sfnt::Tag = *b"name";
const FVAR_TABLE: sfnt::Tag = *b"fvar";
const POST_TABLE: sfnt::Tag = *b"post";
/// The sfnt header: version, table count and binary-search fields; a collection header
/// begins with the same number of bytes before its face offsets.
const SFNT_HEADER_BYTES: usize = 12;
const TABLE_RECORD_BYTES: usize = 16;
const OFFSET_BYTES: usize = 4;
/// Where the table count sits in an sfnt header.
const TABLE_COUNT_OFFSET: usize = 4;

/// Font folders Windows reads that `fontdb::Database::load_system_fonts` may skip: it
/// derives them from `SYSTEMROOT` and `USERPROFILE`.
#[cfg(windows)]
fn platform_directories() -> Vec<PathBuf> {
    [
        std::env::var_os("WINDIR").map(|path| PathBuf::from(path).join("Fonts")),
        std::env::var_os("LOCALAPPDATA")
            .map(|path| PathBuf::from(path).join("Microsoft\\Windows\\Fonts")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Font folders Linux reads that `fontdb::Database::load_system_fonts` may skip: it reads
/// the XDG folders only when fontconfig names none.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
fn platform_directories() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    [
        Some(PathBuf::from("/usr/share/fonts")),
        Some(PathBuf::from("/usr/local/share/fonts")),
        home.as_ref().map(|home| home.join(".local/share/fonts")),
        home.map(|home| home.join(".fonts")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// `fontdb::Database::load_system_fonts` reads every macOS font folder: `/Library/Fonts`,
/// `/System/Library/Fonts` with its subfolders, and `~/Library/Fonts`.
#[cfg(not(any(
    windows,
    all(unix, not(any(target_os = "macos", target_os = "android")))
)))]
fn platform_directories() -> Vec<PathBuf> {
    Vec::new()
}

/// One face of an installed font file.
struct InstalledFace {
    path: Arc<Path>,
    index: u32,
    descriptor: FontDescriptor,
}

/// The faces found in a set of font folders, ordered by file path and face index.
pub(super) struct InstalledFonts {
    faces: Vec<InstalledFace>,
    /// Positions in `faces` by case-folded family name: typographic and legacy family
    /// names, in every language the `name` table gives.
    families: HashMap<String, Vec<usize>>,
    /// The first face with each case-folded full or PostScript name, for `local()`.
    names: HashMap<String, usize>,
}

/// An installed file's bytes, read once for the faces of one book; `None` if unreadable.
type FileData = Arc<OnceLock<Option<Arc<Vec<u8>>>>>;

/// An installed face as one font book loads it.
pub(super) struct InstalledSource {
    pub path: Arc<Path>,
    pub index: u32,
    pub file: FileData,
    /// Whether the face is a font the renderer can shape and embed.
    pub usable: OnceLock<bool>,
}
impl InstalledSource {
    pub(super) fn data(&self) -> Option<&[u8]> {
        if self.usable.get() != Some(&true) {
            return None;
        }
        self.file.get()?.as_deref().map(Vec::as_slice)
    }
}

/// The fonts of `preferences`' folders, scanned once per process for each set of folders;
/// `None` when it names none.
pub(super) fn installed(preferences: &FontPreferences) -> Option<Arc<InstalledFonts>> {
    type Scans = Mutex<HashMap<(bool, Vec<PathBuf>), Arc<InstalledFonts>>>;
    static SCANS: OnceLock<Scans> = OnceLock::new();
    if !preferences.system_fonts && preferences.font_dirs.is_empty() {
        return None;
    }
    let mut scans = SCANS
        .get_or_init(Scans::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let key = (preferences.system_fonts, preferences.font_dirs.clone());
    Some(
        scans
            .entry(key)
            .or_insert_with_key(|(system, directories)| Arc::new(scan(*system, directories)))
            .clone(),
    )
}

/// Finds the faces in the platform's font folders, if `system`, and in `directories`.
/// Files that cannot be read or hold no valid face are skipped.
fn scan(system: bool, directories: &[PathBuf]) -> InstalledFonts {
    work::count(Work::FontScan, 1);
    let mut database = fontdb::Database::new();
    if system {
        database.load_system_fonts();
        for directory in platform_directories() {
            if !covers(&database, &directory) {
                database.load_fonts_dir(directory);
            }
        }
    }
    for directory in directories {
        database.load_fonts_dir(directory);
    }
    let mut found: Vec<(PathBuf, &fontdb::FaceInfo)> = database
        .faces()
        .filter_map(|info| match &info.source {
            fontdb::Source::File(path) => Some((
                std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()),
                info,
            )),
            fontdb::Source::Binary(_) => None,
        })
        .collect();
    found.sort_by(|(path, info), (other_path, other)| {
        (path, info.index).cmp(&(other_path, other.index))
    });
    found.dedup_by(|(path, info), (other_path, other)| {
        path == other_path && info.index == other.index
    });
    let mut fonts = InstalledFonts {
        faces: Vec::with_capacity(found.len()),
        families: HashMap::new(),
        names: HashMap::new(),
    };
    for file in found.chunk_by(|(path, _), (other, _)| path == other) {
        let Some((path, _)) = file.first() else {
            continue;
        };
        let Ok(mut opened) = File::open(path) else {
            continue;
        };
        let path: Arc<Path> = Arc::from(path.as_path());
        for (_, info) in file {
            if let Some(tables) = face_tables(&mut opened, info.index) {
                fonts.add(path.clone(), info, &tables);
            }
        }
    }
    fonts
}

/// Whether a face `database` holds lies in `directory`, which it then has read.
fn covers(database: &fontdb::Database, directory: &Path) -> bool {
    let canonical = std::fs::canonicalize(directory).ok();
    database.faces().any(|info| match &info.source {
        fontdb::Source::File(path) => {
            path.starts_with(directory)
                || canonical
                    .as_deref()
                    .is_some_and(|canonical| path.starts_with(canonical))
        }
        fontdb::Source::Binary(_) => false,
    })
}

/// The tables of a face that `fontdb` does not keep, read without the rest of its file.
#[derive(Default)]
struct FaceTables {
    name: Option<Vec<u8>>,
    fvar: Option<Vec<u8>>,
    post: Option<Vec<u8>>,
}

fn read_at(file: &mut File, offset: u64, length: usize) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buffer = vec![0; length];
    file.read_exact(&mut buffer).ok()?;
    Some(buffer)
}

/// The `name`, `fvar` and `post` tables of face `index` of the sfnt font or collection `file`.
fn face_tables(file: &mut File, index: u32) -> Option<FaceTables> {
    let file_length = file.metadata().ok()?.len();
    let header = read_at(file, 0, SFNT_HEADER_BYTES)?;
    let start = if header.first_chunk() == Some(&sfnt::COLLECTION_TAG) {
        let entry = SFNT_HEADER_BYTES as u64 + u64::from(index) * OFFSET_BYTES as u64;
        let offset = read_at(file, entry, OFFSET_BYTES)?;
        u64::from(sfnt::Reader::new(&offset, "collection offset").u32().ok()?)
    } else {
        0
    };
    let header = if start == 0 {
        header
    } else {
        read_at(file, start, SFNT_HEADER_BYTES)?
    };
    let count = usize::from(sfnt::u16_at(&header, TABLE_COUNT_OFFSET)?);
    let records = read_at(
        file,
        start + SFNT_HEADER_BYTES as u64,
        count * TABLE_RECORD_BYTES,
    )?;
    let mut reader = sfnt::Reader::new(&records, "table directory");
    let mut tables = FaceTables::default();
    for _ in 0..count {
        let tag = reader.tag().ok()?;
        let (_checksum, offset, length) =
            (reader.u32().ok()?, reader.u32().ok()?, reader.u32().ok()?);
        let slot = match tag {
            NAME_TABLE => &mut tables.name,
            FVAR_TABLE => &mut tables.fvar,
            POST_TABLE => &mut tables.post,
            _ => continue,
        };
        if u64::from(offset) + u64::from(length) <= file_length {
            *slot = read_at(file, u64::from(offset), usize::try_from(length).ok()?);
        }
    }
    Some(tables)
}

impl InstalledFonts {
    /// Records a face `fontdb` read, with the names, axes and slant of its own tables.
    fn add(&mut self, path: Arc<Path>, info: &fontdb::FaceInfo, tables: &FaceTables) {
        let names = tables
            .name
            .as_deref()
            .and_then(ttf_parser::name::Table::parse);
        let records = || {
            names
                .into_iter()
                .flat_map(|table| table.names)
                .filter_map(|record| Some((record.name_id, record.to_string()?)))
        };
        let families: Vec<String> = info
            .families
            .iter()
            .map(|(family, _)| family.clone())
            .chain(
                records()
                    .filter(|(id, _)| {
                        matches!(
                            *id,
                            ttf_parser::name_id::TYPOGRAPHIC_FAMILY | ttf_parser::name_id::FAMILY
                        )
                    })
                    .map(|(_, family)| family),
            )
            .collect();
        let Some(primary) = families.first() else {
            return;
        };
        let axes: Vec<ttf_parser::VariationAxis> = tables
            .fvar
            .as_deref()
            .and_then(ttf_parser::fvar::Table::parse)
            .map(|table| table.axes.into_iter().collect())
            .unwrap_or_default();
        let axis = |tag| axes.iter().find(|axis| axis.tag == tag);
        let weight = match axis(WEIGHT_AXIS) {
            Some(axis) => FontRange::new(weight(axis.min_value), weight(axis.max_value)),
            None => FontRange::single(weight(f32::from(info.weight.0))),
        };
        let width = match axis(WIDTH_AXIS) {
            Some(axis) => FontRange::new(width(axis.min_value), width(axis.max_value)),
            None => FontRange::single(stretch(info.stretch)),
        };
        let angle = tables
            .post
            .as_deref()
            .and_then(ttf_parser::post::Table::parse)
            .map_or(0.0, |post| post.italic_angle);
        let style = match info.style {
            // `post` gives the angle counter-clockwise; CSS slants and `left` clockwise.
            fontdb::Style::Italic if angle < 0.0 => FontFaceStyle::Left,
            fontdb::Style::Italic if angle > 0.0 => FontFaceStyle::Right,
            fontdb::Style::Italic => FontFaceStyle::Italic,
            fontdb::Style::Oblique => FontFaceStyle::Oblique(FontRange::single(
                FontSlant::new(-angle)
                    .ok()
                    .filter(|slant| *slant != FontSlant::NORMAL)
                    .unwrap_or(FontSlant::DEFAULT),
            )),
            fontdb::Style::Normal if axis(ITALIC_AXIS).or(axis(SLANT_AXIS)).is_some() => {
                FontFaceStyle::Auto
            }
            fontdb::Style::Normal => FontFaceStyle::Normal,
        };
        let position = self.faces.len();
        self.faces.push(InstalledFace {
            path,
            index: info.index,
            descriptor: FontDescriptor {
                family: primary.clone(),
                weight,
                width,
                style,
            },
        });
        for family in &families {
            let faces = self.families.entry(matching::folded(family)).or_default();
            if faces.last() != Some(&position) {
                faces.push(position);
            }
        }
        let local_names = records()
            .filter(|(id, _)| {
                matches!(
                    *id,
                    ttf_parser::name_id::FULL_NAME | ttf_parser::name_id::POST_SCRIPT_NAME
                )
            })
            .map(|(_, name)| name)
            .chain([info.post_script_name.clone()]);
        for name in local_names {
            self.names
                .entry(matching::folded(&name))
                .or_insert(position);
        }
    }

    /// Positions of the faces named `family`.
    pub(super) fn family(&self, family: &str) -> &[usize] {
        self.families
            .get(&matching::folded(family))
            .map_or(&[], Vec::as_slice)
    }

    /// The position of the first face whose full or PostScript name is `name` (CSS Fonts 4
    /// §4.3.1).
    pub(super) fn named(&self, name: &str) -> Option<usize> {
        self.names.get(&matching::folded(name)).copied()
    }

    /// The faces as a font book holds them, unread, one file's faces sharing its bytes.
    pub(super) fn book_faces(&self) -> Vec<FontFace> {
        let mut file: Option<(&Path, FileData)> = None;
        self.faces
            .iter()
            .map(|face| {
                let shared = match &file {
                    Some((path, shared)) if *path == &*face.path => shared.clone(),
                    _ => {
                        let shared = Arc::new(OnceLock::new());
                        file = Some((&face.path, shared.clone()));
                        shared
                    }
                };
                FontFace {
                    descriptor: face.descriptor.clone(),
                    settings: FaceSettings::default(),
                    data: FontData::Installed(Arc::new(InstalledSource {
                        path: face.path.clone(),
                        index: face.index,
                        file: shared,
                        usable: OnceLock::new(),
                    })),
                    namespace: FontNamespace::ROOT,
                }
            })
            .collect()
    }
}

/// A weight from a font, within the range CSS accepts.
fn weight(value: f32) -> FontWeight {
    FontWeight::new(value.clamp(FontWeight::MIN.value(), FontWeight::MAX.value()))
        .unwrap_or(FontWeight::NORMAL)
}

/// A width percentage from a font's `wdth` axis.
fn width(value: f32) -> FontWidth {
    FontWidth::new(value).unwrap_or(FontWidth::NORMAL)
}

/// The width an OS/2 `usWidthClass` names (CSS Fonts 4 §2.2 table).
fn stretch(stretch: fontdb::Stretch) -> FontWidth {
    match stretch {
        fontdb::Stretch::UltraCondensed => FontWidth::ULTRA_CONDENSED,
        fontdb::Stretch::ExtraCondensed => FontWidth::EXTRA_CONDENSED,
        fontdb::Stretch::Condensed => FontWidth::CONDENSED,
        fontdb::Stretch::SemiCondensed => FontWidth::SEMI_CONDENSED,
        fontdb::Stretch::Normal => FontWidth::NORMAL,
        fontdb::Stretch::SemiExpanded => FontWidth::SEMI_EXPANDED,
        fontdb::Stretch::Expanded => FontWidth::EXPANDED,
        fontdb::Stretch::ExtraExpanded => FontWidth::EXTRA_EXPANDED,
        fontdb::Stretch::UltraExpanded => FontWidth::ULTRA_EXPANDED,
    }
}
