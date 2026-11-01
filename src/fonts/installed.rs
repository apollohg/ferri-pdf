//! Fonts installed on the system or in configured directories. A scan reads only each file's
//! headers and the tables that describe its faces; a face's file is read when rendering first
//! uses it.
use crate::fonts::loading::{ITALIC_AXIS, SLANT_AXIS, WEIGHT_AXIS, WIDTH_AXIS};
use crate::fonts::{
    FaceSettings, FontData, FontDescriptor, FontFace, FontFaceStyle, FontNamespace,
    FontPreferences, FontRange, FontSlant, FontWeight, FontWidth, matching, sfnt,
};
use crate::work::{self, Work};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

const NAME_TABLE: sfnt::Tag = *b"name";
const OS2_TABLE: sfnt::Tag = *b"OS/2";
const FVAR_TABLE: sfnt::Tag = *b"fvar";
const POST_TABLE: sfnt::Tag = *b"post";
/// The sfnt header: version, table count and binary-search fields; a collection header
/// begins with the same number of bytes before its face offsets.
const SFNT_HEADER_BYTES: usize = 12;
const TABLE_RECORD_BYTES: usize = 16;
const OFFSET_BYTES: usize = 4;
/// Where the table count sits in an sfnt header.
const TABLE_COUNT_OFFSET: usize = 4;
/// Where the face count sits in a collection header.
const FACE_COUNT_OFFSET: usize = 8;
/// The extensions of the files a folder scan reads, matched as `fontdb` matches them.
const FONT_EXTENSIONS: [&str; 8] = ["ttf", "ttc", "TTF", "TTC", "otf", "otc", "OTF", "OTC"];

#[cfg(windows)]
const WINDOWS_DEFAULT_FONTS: &str = "C:\\Windows\\Fonts\\";
#[cfg(windows)]
const WINDOWS_FONTS: &str = "Fonts";
#[cfg(windows)]
const WINDOWS_USER_FONTS: &str = "Microsoft\\Windows\\Fonts";
#[cfg(windows)]
const WINDOWS_PROFILE_APP_DATA: [&str; 2] = ["AppData\\Local", "AppData\\Roaming"];

/// The Windows font folders: those `fontdb` reads, found from `SYSTEMROOT` and `USERPROFILE`,
/// and those `WINDIR` and `LOCALAPPDATA` name.
#[cfg(windows)]
fn system_directories() -> Vec<PathBuf> {
    let variable = |name| std::env::var_os(name).map(PathBuf::from);
    let profile = variable("USERPROFILE");
    let profile_fonts = WINDOWS_PROFILE_APP_DATA.iter().filter_map(|app_data| {
        profile
            .as_ref()
            .map(|profile| profile.join(app_data).join(WINDOWS_USER_FONTS))
    });
    [
        Some(variable("SYSTEMROOT").map_or_else(
            || PathBuf::from(WINDOWS_DEFAULT_FONTS),
            |root| root.join(WINDOWS_FONTS),
        )),
        variable("WINDIR").map(|path| path.join(WINDOWS_FONTS)),
        variable("LOCALAPPDATA").map(|path| path.join(WINDOWS_USER_FONTS)),
    ]
    .into_iter()
    .flatten()
    .chain(profile_fonts)
    .collect()
}

#[cfg(target_os = "macos")]
const MACOS_DIRECTORIES: [&str; 3] = [
    "/Library/Fonts",
    "/System/Library/Fonts",
    "/Network/Library/Fonts",
];
/// Where macOS keeps downloadable fonts, in asset folders whose names start with
/// `MACOS_FONT_ASSETS`.
#[cfg(target_os = "macos")]
const MACOS_ASSETS: &str = "/System/Library/AssetsV2";
#[cfg(target_os = "macos")]
const MACOS_FONT_ASSETS: &str = "com_apple_MobileAsset_Font";
#[cfg(target_os = "macos")]
const MACOS_USER_FONTS: &str = "Library/Fonts";

/// The macOS font folders `fontdb` reads: the system's, its downloadable fonts' and the
/// user's.
#[cfg(target_os = "macos")]
fn system_directories() -> Vec<PathBuf> {
    let assets = std::fs::read_dir(MACOS_ASSETS)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(MACOS_FONT_ASSETS)
        })
        .map(|entry| entry.path());
    MACOS_DIRECTORIES
        .iter()
        .map(PathBuf::from)
        .chain(assets)
        .chain(home().map(|home| home.join(MACOS_USER_FONTS)))
        .collect()
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
const LINUX_DIRECTORIES: [&str; 2] = ["/usr/share/fonts", "/usr/local/share/fonts"];
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
const LINUX_USER_DIRECTORIES: [&str; 2] = [".local/share/fonts", ".fonts"];
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
const REDOX_DIRECTORY: &str = "/ui/fonts";
/// The user's fontconfig file, in the XDG configuration folder.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
const FONTCONFIG_USER_FILE: &str = "fontconfig/fonts.conf";
/// Read only when the user's file cannot be.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
const FONTCONFIG_LOCAL_FILE: &str = "/etc/fonts/local.conf";
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
const FONTCONFIG_FILE: &str = "/etc/fonts/fonts.conf";
/// The XDG configuration folder within the home folder when `XDG_CONFIG_HOME` is unset.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
const XDG_CONFIG_DEFAULT: &str = ".config";
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
const HOME_PREFIX: &str = "~";

/// The folders fontconfig's configuration names, read as `fontdb` reads it, then the system
/// and user folders `fontdb` reads only when that configuration names none, and on Redox
/// its font folder.
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
fn system_directories() -> Vec<PathBuf> {
    let home = home();
    let mut config = fontconfig_parser::FontConfig::default();
    // A configuration file that is missing or malformed names no folders, as in fontconfig.
    if let Some(file) = std::env::var_os("FONTCONFIG_FILE") {
        config.merge_config(Path::new(&file)).ok();
    } else {
        let user_file = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(XDG_CONFIG_DEFAULT)))
            .map(|config_home| config_home.join(FONTCONFIG_USER_FILE));
        if user_file.is_none_or(|file| config.merge_config(&file).is_err()) {
            config.merge_config(FONTCONFIG_LOCAL_FILE).ok();
        }
        config.merge_config(FONTCONFIG_FILE).ok();
    }
    let configured = config.dirs.into_iter().filter_map(|directory| {
        match directory.path.strip_prefix(HOME_PREFIX) {
            Ok(relative) => home.as_ref().map(|home| home.join(relative)),
            Err(_) => Some(directory.path),
        }
    });
    let user = home
        .iter()
        .flat_map(|home| LINUX_USER_DIRECTORIES.map(|directory| home.join(directory)));
    configured
        .chain(LINUX_DIRECTORIES.iter().map(PathBuf::from))
        .chain(user)
        .chain(cfg!(target_os = "redox").then(|| PathBuf::from(REDOX_DIRECTORY)))
        .collect()
}

/// `fontdb` reads no font folders on other systems.
#[cfg(not(any(
    windows,
    target_os = "macos",
    all(unix, not(any(target_os = "macos", target_os = "android")))
)))]
fn system_directories() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(all(unix, not(target_os = "android")))]
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `path` with links resolved, or as given if it cannot be.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Adds the font files in `directory` and its subfolders to `files`, following links and
/// judging a link by its target's name, as `fontdb` does. Folders in `visited` are not read
/// again, so a link back to an enclosing folder ends the walk there.
fn find_files(directory: &Path, visited: &mut HashSet<PathBuf>, files: &mut Vec<PathBuf>) {
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        if !visited.insert(canonical(&directory)) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let (path, kind) = if kind.is_symlink() {
                let Ok(target) = std::fs::canonicalize(entry.path()) else {
                    continue;
                };
                let Ok(metadata) = std::fs::metadata(&target) else {
                    continue;
                };
                (target, metadata.file_type())
            } else {
                (entry.path(), kind)
            };
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file()
                && path
                    .extension()
                    .and_then(OsStr::to_str)
                    .is_some_and(|extension| FONT_EXTENSIONS.contains(&extension))
            {
                files.push(path);
            }
        }
    }
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
    let system_directories = if system {
        system_directories()
    } else {
        Vec::new()
    };
    let mut visited = HashSet::new();
    let mut files = Vec::new();
    for directory in system_directories.iter().chain(directories) {
        find_files(directory, &mut visited, &mut files);
    }
    let mut files: Vec<PathBuf> = files.iter().map(|path| canonical(path)).collect();
    files.sort();
    files.dedup();
    let mut database = fontdb::Database::new();
    let mut fonts = InstalledFonts {
        faces: Vec::new(),
        families: HashMap::new(),
        names: HashMap::new(),
    };
    for path in files {
        let Ok(mut file) = File::open(&path) else {
            continue;
        };
        let path: Arc<Path> = Arc::from(path);
        for (index, tables) in file_faces(&mut file) {
            if let Some(info) = face_info(&mut database, &tables) {
                fonts.add(path.clone(), index, &info, &tables);
            }
        }
    }
    fonts
}

/// The tables of a face a scan reads: those `fontdb` describes a face from, and `fvar`.
#[derive(Default)]
struct FaceTables {
    name: Option<Vec<u8>>,
    os2: Option<Vec<u8>>,
    fvar: Option<Vec<u8>>,
    post: Option<Vec<u8>>,
}

/// `length` bytes of `file` from `offset`.
fn read_at(file: &mut File, offset: u64, length: usize) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut buffer = vec![0; length];
    file.read_exact(&mut buffer)?;
    work::count(Work::FontScanBytes, length);
    Ok(buffer)
}

/// The faces of the sfnt font or collection `file`, by index, with their tables. A face
/// whose header or table directory `ttf_parser::RawFace::parse` rejects is skipped, as
/// `fontdb` skips it.
fn file_faces(file: &mut File) -> Vec<(u32, FaceTables)> {
    let Ok(length) = file.metadata().map(|metadata| metadata.len()) else {
        return Vec::new();
    };
    let Ok(header) = read_at(file, 0, SFNT_HEADER_BYTES) else {
        return Vec::new();
    };
    if header.first_chunk() != Some(&sfnt::COLLECTION_TAG) {
        return face_tables(file, length, 0, header)
            .map(|tables| vec![(0, tables)])
            .unwrap_or_default();
    }
    let Some(count) = header
        .get(FACE_COUNT_OFFSET..)
        .and_then(<[u8]>::first_chunk)
        .map(|count| u32::from_be_bytes(*count))
    else {
        return Vec::new();
    };
    let offsets_size = u64::from(count) * OFFSET_BYTES as u64;
    let directory_end = SFNT_HEADER_BYTES as u64 + offsets_size;
    if directory_end > length {
        return Vec::new();
    }
    let Some(offsets) = usize::try_from(offsets_size)
        .ok()
        .and_then(|size| read_at(file, SFNT_HEADER_BYTES as u64, size).ok())
    else {
        return Vec::new();
    };
    offsets
        .as_chunks::<OFFSET_BYTES>()
        .0
        .iter()
        .zip(0..)
        .filter_map(|(offset, index)| {
            let start = u64::from(u32::from_be_bytes(*offset));
            if start < directory_end {
                return None;
            }
            let header = read_at(file, start, SFNT_HEADER_BYTES).ok()?;
            if header.first_chunk() == Some(&sfnt::COLLECTION_TAG) {
                return None;
            }
            Some((index, face_tables(file, length, start, header)?))
        })
        .collect()
}

/// The tables of the face whose sfnt `header` starts at `start` in `file`, `length` bytes
/// long. A table is found as `ttf_parser::RawFace::table` finds it, and is absent if it does
/// not lie within the file; `None` if the table directory is invalid or reading fails.
fn face_tables(file: &mut File, length: u64, start: u64, header: Vec<u8>) -> Option<FaceTables> {
    let count = usize::from(sfnt::u16_at(&header, TABLE_COUNT_OFFSET)?);
    let mut directory = header;
    directory.extend(
        read_at(
            file,
            start + SFNT_HEADER_BYTES as u64,
            count * TABLE_RECORD_BYTES,
        )
        .ok()?,
    );
    let face = ttf_parser::RawFace::parse(&directory, 0).ok()?;
    let mut table = |tag: sfnt::Tag| -> std::io::Result<Option<Vec<u8>>> {
        let tag = ttf_parser::Tag::from_bytes(&tag);
        let Some((_, record)) = face
            .table_records
            .binary_search_by(|record| record.tag.cmp(&tag))
        else {
            return Ok(None);
        };
        let offset = u64::from(record.offset);
        if offset + u64::from(record.length) > length {
            return Ok(None);
        }
        let size = usize::try_from(record.length).map_err(std::io::Error::other)?;
        read_at(file, offset, size).map(Some)
    };
    Some(FaceTables {
        name: table(NAME_TABLE).ok()?,
        os2: table(OS2_TABLE).ok()?,
        fvar: table(FVAR_TABLE).ok()?,
        post: table(POST_TABLE).ok()?,
    })
}

/// The face `fontdb` describes from `tables`: its families, PostScript name, style, weight
/// and width; `None` for a face it rejects. It parses a font of only the tables it reads.
fn face_info(database: &mut fontdb::Database, tables: &FaceTables) -> Option<fontdb::FaceInfo> {
    let font = sfnt::build(
        [
            (NAME_TABLE, &tables.name),
            (OS2_TABLE, &tables.os2),
            (POST_TABLE, &tables.post),
        ]
        .into_iter()
        .filter_map(|(tag, table)| Some((tag, Cow::Borrowed(table.as_deref()?)))),
    );
    let id = *database
        .load_font_source(fontdb::Source::Binary(Arc::new(font)))
        .first()?;
    let info = database.face(id).cloned();
    database.remove_face(id);
    info
}

impl InstalledFonts {
    /// Records face `index` of the file at `path` as `fontdb` describes it, with the names,
    /// axes and slant of its own tables.
    fn add(&mut self, path: Arc<Path>, index: u32, info: &fontdb::FaceInfo, tables: &FaceTables) {
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
            index,
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
