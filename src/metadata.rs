use std::fs;
use std::fmt;
use std::path::{self, Path, PathBuf};
use std::ffi::OsStr;
use std::cmp::Ordering;
use std::collections::{BTreeSet, BTreeMap};
use fnv::{FnvHashMap, FnvHashSet};
use chrono::{Local, DateTime};
use serde::{Serialize, Deserialize};
use lazy_static::lazy_static;
use regex::Regex;
use failure::{Error, ResultExt};
use crate::document::{Document, SimpleTocEntry, TextLocation};
use crate::document::epub::EpubDocument;
use crate::helpers::simple_date_format;
use crate::settings::{ImportSettings, CategoryProvider};
use crate::document::file_kind;
use crate::symbolic_path;

pub const METADATA_FILENAME: &str = ".metadata.json";
pub const IMPORTED_MD_FILENAME: &str = ".metadata-imported.json";
pub const TRASH_NAME: &str = ".trash";

pub const DEFAULT_CONTRAST_EXPONENT: f32 = 1.0;
pub const DEFAULT_CONTRAST_GRAY: f32 = 224.0;

pub type Metadata = Vec<Info>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Info {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub subtitle: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub year: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub language: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub publisher: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub series: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub edition: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub volume: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub number: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isbn: String,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub categories: BTreeSet<String>,
    pub file: FileInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reader: Option<ReaderInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toc: Option<Vec<SimpleTocEntry>>,
    #[serde(with = "simple_date_format")]
    pub added: DateTime<Local>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FileInfo {
    pub path: PathBuf,
    pub kind: String,
    pub size: u64,
}

impl Default for FileInfo {
    fn default() -> Self {
        FileInfo {
            path: PathBuf::default(),
            kind: String::default(),
            size: u64::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Annotation {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    pub selection: [TextLocation; 2],
    #[serde(with = "simple_date_format")]
    pub modified: DateTime<Local>,
}

impl Default for Annotation {
    fn default() -> Self {
        Annotation {
            note: String::new(),
            text: String::new(),
            selection: [TextLocation::Dynamic(0), TextLocation::Dynamic(1)],
            modified: Local::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Margin {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Margin {
    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Margin {
        Margin { top, right, bottom, left }
    }
}

impl Default for Margin {
    fn default() -> Margin {
        Margin::new(0.0, 0.0, 0.0, 0.0)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PageScheme {
    Any,
    EvenOdd,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CroppingMargins {
    Any(Margin),
    EvenOdd([Margin; 2]),
}

impl CroppingMargins {
    pub fn margin(&self, index: usize) -> &Margin {
        match *self {
            CroppingMargins::Any(ref margin) => margin,
            CroppingMargins::EvenOdd(ref pair) => &pair[index % 2],
        }
    }

    pub fn margin_mut(&mut self, index: usize) -> &mut Margin {
        match *self {
            CroppingMargins::Any(ref mut margin) => margin,
            CroppingMargins::EvenOdd(ref mut pair) => &mut pair[index % 2],
        }
    }

    pub fn apply(&mut self, index: usize, scheme: PageScheme) {
        let margin = self.margin(index).clone();

        match scheme {
            PageScheme::Any => *self = CroppingMargins::Any(margin),
            PageScheme::EvenOdd => *self = CroppingMargins::EvenOdd([margin.clone(), margin]),
        }
    }

    pub fn is_split(&self) -> bool {
        match *self {
            CroppingMargins::Any(..) => false,
            _ => true,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum TextAlign {
    Justify,
    Left,
    Right,
    Center,
}

impl TextAlign {
    pub fn icon_name(&self) -> &str {
        match self {
            TextAlign::Justify => "align-justify",
            TextAlign::Left => "align-left",
            TextAlign::Right => "align-right",
            TextAlign::Center => "align-center",
        }
    }
}

impl fmt::Display for TextAlign {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReaderInfo {
    #[serde(with = "simple_date_format")]
    pub opened: DateTime<Local>,
    pub current_page: usize,
    pub pages_count: usize,
    pub finished: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom_mode: Option<ZoomMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_offset: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<i8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cropping_margins: Option<CroppingMargins>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen_margin_width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_align: Option<TextAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contrast_exponent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contrast_gray: Option<f32>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub page_names: BTreeMap<usize, String>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub bookmarks: BTreeSet<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub enum ZoomMode {
    FitToPage,
    FitToWidth,
}

impl ReaderInfo {
    pub fn progress(&self) -> f32 {
        (self.current_page / self.pages_count) as f32
    }
}

impl Default for ReaderInfo {
    fn default() -> Self {
        ReaderInfo {
            opened: Local::now(),
            current_page: 0,
            pages_count: 1,
            finished: false,
            zoom_mode: None,
            top_offset: None,
            rotation: None,
            cropping_margins: None,
            margin_width: None,
            screen_margin_width: None,
            font_family: None,
            font_size: None,
            text_align: None,
            line_height: None,
            contrast_exponent: None,
            contrast_gray: None,
            page_names: BTreeMap::new(),
            bookmarks: BTreeSet::new(),
            annotations: Vec::new(),
        }
    }
}

impl Default for Info {
    fn default() -> Self {
        Info {
            title: String::default(),
            subtitle: String::default(),
            author: String::default(),
            year: String::default(),
            language: String::default(),
            publisher: String::default(),
            series: String::default(),
            edition: String::default(),
            volume: String::default(),
            number: String::default(),
            isbn: String::default(),
            categories: BTreeSet::new(),
            file: FileInfo::default(),
            added: Local::now(),
            reader: None,
            toc: None,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Status {
    New,
    Reading(f32),
    Finished,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum SimpleStatus {
    New,
    Reading,
    Finished,
}

impl fmt::Display for SimpleStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl Info {
    pub fn status(&self) -> Status {
        if let Some(ref r) = self.reader {
            if r.finished {
                Status::Finished
            } else {
                Status::Reading(r.current_page as f32 / r.pages_count as f32)
            }
        } else {
            Status::New
        }
    }

    pub fn simple_status(&self) -> SimpleStatus {
        if let Some(ref r) = self.reader {
            if r.finished {
                SimpleStatus::Finished
            } else {
                SimpleStatus::Reading
            }
        } else {
            SimpleStatus::New
        }
    }

    pub fn file_stem(&self) -> String {
        self.file.path.file_stem().unwrap().to_string_lossy().into_owned()
    }

    pub fn author(&self) -> &str {
        if self.author.is_empty() {
            "Unknown Author"
        } else {
            &self.author
        }
    }

    pub fn title(&self) -> String {
        if self.title.is_empty() {
            return self.file_stem();
        }

        let mut title = self.title.clone();

        if !self.number.is_empty() && self.series.is_empty() {
            title = format!("{} #{}", title, self.number);
        }

        if !self.volume.is_empty() {
            title = format!("{} — vol. {}", title, self.volume);
        }

        if !self.subtitle.is_empty() {
            title = if self.subtitle.chars().next().unwrap().is_alphanumeric() &&
                       title.chars().last().unwrap().is_alphanumeric() {
                format!("{}: {}", title, self.subtitle)
            } else {
                format!("{} {}", title, self.subtitle)
            };
        }

        if !self.series.is_empty() && !self.number.is_empty() {
            title = format!("{} ({} #{})", title, self.series, self.number);
        }

        title
    }

    #[inline]
    pub fn is_match(&self, query: &Option<Regex>) -> bool {
        if let Some(ref query) = *query {
            query.is_match(&self.title) ||
            query.is_match(&self.subtitle) ||
            query.is_match(&self.author) ||
            query.is_match(&self.series) ||
            self.categories.iter().any(|c| query.is_match(c)) ||
            self.file.path.to_str().map(|s| query.is_match(s)).unwrap_or(false)
        } else {
            true
        }
    }

    // TODO: handle the following case: *Walter M. Miller Jr.*?
    // NOTE: e.g.: John Le Carré: the space between *Le* and *Carré* is a non-breaking space
    pub fn alphabetic_author(&self) -> &str {
        self.author().split(',').next()
                     .and_then(|a| a.split(' ').last())
                     .unwrap_or_default()
    }

    pub fn alphabetic_title(&self) -> &str {
        let mut start = 0;
        if let Some(re) = TITLE_PREFIXES.get(self.language.as_str()) {
            if let Some(m) = re.find(&self.title) {
                start = m.end()
            }
        }
        &self.title[start..]
    }

    pub fn label(&self) -> String {
        format!("{} · {}", self.title(), self.author())
    }
}

pub fn make_query(text: &str) -> Option<Regex> {
    let any = Regex::new(r"^(\.*|\s)$").unwrap();

    if any.is_match(text) {
        return None;
    }

    let text = text.replace('a', "[aáàâä]")
                   .replace('e', "[eéèêë]")
                   .replace('i', "[iíìîï]")
                   .replace('o', "[oóòôö]")
                   .replace('u', "[uúùûü]")
                   .replace('c', "[cç]")
                   .replace("ae", "(ae|æ)")
                   .replace("oe", "(oe|œ)");
    Regex::new(&format!("(?i){}", text))
          .map_err(|e| eprintln!("{}", e))
          .ok()
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SortMethod {
    Opened,
    Added,
    Progress,
    Author,
    Title,
    Year,
    Size,
    Kind,
    Pages,
    FileName,
    FilePath,
}

impl SortMethod {
    pub fn reverse_order(self) -> bool {
        match self {
            SortMethod::Author |
            SortMethod::Title |
            SortMethod::Kind |
            SortMethod::FileName |
            SortMethod::FilePath => false,
            _ => true,
        }
    }

    pub fn label(&self) -> &str {
        match *self {
            SortMethod::Opened => "Date Opened",
            SortMethod::Added => "Date Added",
            SortMethod::Progress => "Progress",
            SortMethod::Author => "Author",
            SortMethod::Title => "Title",
            SortMethod::Year => "Year",
            SortMethod::Size => "File Size",
            SortMethod::Kind => "File Type",
            SortMethod::Pages => "Pages",
            SortMethod::FileName => "File Name",
            SortMethod::FilePath => "File Path",
        }
    }

    pub fn title(self) -> String {
        format!("Sort by: {}", self.label())
    }
}

pub fn sort(md: &mut Metadata, sort_method: SortMethod, reverse_order: bool) {
    let sort_fn: fn(&Info, &Info) -> Ordering = match sort_method {
        SortMethod::Opened => sort_opened,
        SortMethod::Added => sort_added,
        SortMethod::Progress => sort_progress,
        SortMethod::Author => sort_author,
        SortMethod::Title => sort_title,
        SortMethod::Year => sort_year,
        SortMethod::Size => sort_size,
        SortMethod::Kind => sort_kind,
        SortMethod::Pages => sort_pages,
        SortMethod::FileName => sort_filename,
        SortMethod::FilePath => sort_filepath,
    };
    if reverse_order {
        md.sort_by(|a, b| sort_fn(a, b).reverse());
    } else {
        md.sort_by(sort_fn);
    }
}

pub fn sort_opened(i1: &Info, i2: &Info) -> Ordering {
    match (&i1.reader, &i2.reader) {
        (&None, &None) => Ordering::Equal,
        (&None, &Some(_)) => Ordering::Less,
        (&Some(_), &None) => Ordering::Greater,
        (&Some(ref r1), &Some(ref r2)) => r1.opened.cmp(&r2.opened),
    }
}

pub fn sort_pages(i1: &Info, i2: &Info) -> Ordering {
    match (&i1.reader, &i2.reader) {
        (&None, &None) => Ordering::Equal,
        (&None, &Some(_)) => Ordering::Less,
        (&Some(_), &None) => Ordering::Greater,
        (&Some(ref r1), &Some(ref r2)) => r1.pages_count.cmp(&r2.pages_count),
    }
}

pub fn sort_added(i1: &Info, i2: &Info) -> Ordering {
    i1.added.cmp(&i2.added)
}

// FIXME: 'Z'.cmp('É') equals Ordering::Less
pub fn sort_author(i1: &Info, i2: &Info) -> Ordering {
    i1.alphabetic_author().cmp(i2.alphabetic_author())
}

pub fn sort_title(i1: &Info, i2: &Info) -> Ordering {
    i1.alphabetic_title().cmp(i2.alphabetic_title())
}

// Ordering: Finished < New < Reading
pub fn sort_progress(i1: &Info, i2: &Info) -> Ordering {
    match (i1.status(), i2.status()) {
        (Status::Finished, Status::Finished) => Ordering::Equal,
        (Status::New, Status::New) => Ordering::Equal,
        (Status::New, Status::Finished) => Ordering::Greater,
        (Status::Finished, Status::New) => Ordering::Less,
        (Status::New, Status::Reading(..)) => Ordering::Less,
        (Status::Reading(..), Status::New) => Ordering::Greater,
        (Status::Finished, Status::Reading(..)) => Ordering::Less,
        (Status::Reading(..), Status::Finished) => Ordering::Greater,
        (Status::Reading(p1), Status::Reading(p2)) => p1.partial_cmp(&p2)
                                                        .unwrap_or(Ordering::Equal),
    }
}

pub fn sort_size(i1: &Info, i2: &Info) -> Ordering {
    i1.file.size.cmp(&i2.file.size)
}

pub fn sort_kind(i1: &Info, i2: &Info) -> Ordering {
    i1.file.kind.cmp(&i2.file.kind)
}

pub fn sort_year(i1: &Info, i2: &Info) -> Ordering {
    i1.year.cmp(&i2.year)
}

pub fn sort_filename(i1: &Info, i2: &Info) -> Ordering {
    i1.file.path.file_name().cmp(&i2.file.path.file_name())
}

pub fn sort_filepath(i1: &Info, i2: &Info) -> Ordering {
    i1.file.path.cmp(&i2.file.path)
}

lazy_static! {
    pub static ref TITLE_PREFIXES: FnvHashMap<&'static str, Regex> = {
        let mut p = FnvHashMap::default();
        p.insert("", Regex::new(r"^(The|An?)\s").unwrap());
        p.insert("french", Regex::new(r"^(Les?\s|La\s|L['’]|Une?\s|Des?\s|Du\s)").unwrap());
        p
    };

    pub static ref RESERVED_DIRECTORIES: FnvHashSet<&'static str> = [
        TRASH_NAME,
    ].iter().cloned().collect();
}

pub fn auto_import(dir: &Path, metadata: &Metadata, settings: &ImportSettings) -> Result<Metadata, Error> {
    let mut imported_metadata = import(dir, metadata, settings)?;
    extract_metadata_from_epub(dir, &mut imported_metadata, settings);
    Ok(imported_metadata)
}

pub fn import(dir: &Path, metadata: &Metadata, settings: &ImportSettings) -> Result<Metadata, Error> {
    let files = find_files(dir, dir, settings.traverse_hidden)?;
    let known: FnvHashSet<PathBuf> = metadata.iter()
                                             .map(|info| info.file.path.clone())
                                             .collect();
    let mut metadata = Vec::new();
    let path_as_category = settings.category_providers.contains(&CategoryProvider::Path);

    for file_info in &files {
        if !known.contains(&file_info.path) && settings.allowed_kinds.contains(&file_info.kind) {
            println!("{}", file_info.path.display());
            let mut info = Info::default();
            info.file = file_info.clone();
            if path_as_category {
                if let Some(p) = info.file.path.parent() {
                    let categ = p.to_string_lossy()
                                 .replace(symbolic_path::PATH_SEPARATOR, "")
                                 .replace(path::MAIN_SEPARATOR, &symbolic_path::PATH_SEPARATOR.to_string());
                    if !categ.is_empty() {
                        info.categories = [categ].iter().cloned().collect();
                    }
                }
            }
            metadata.push(info);
        }
    }

    Ok(metadata)
}

pub fn extract_metadata_from_epub(dir: &Path, metadata: &mut Metadata, settings: &ImportSettings) {
    let subjects_as_categories = settings.category_providers.contains(&CategoryProvider::Subject);

    for info in metadata {
        if !info.title.is_empty() || info.file.kind != "epub" {
            continue;
        }

        let path = dir.join(&info.file.path);

        match EpubDocument::new(&path) {
            Ok(doc) => {
                info.title = doc.title().unwrap_or_default();
                info.author = doc.author().unwrap_or_default();
                info.year = doc.year().unwrap_or_default();
                info.publisher = doc.publisher().unwrap_or_default();
                info.series = doc.series().unwrap_or_default();
                if !info.series.is_empty() {
                    info.number = doc.series_index().unwrap_or_default();
                }
                info.language = doc.language().unwrap_or_default();
                if subjects_as_categories {
                    info.categories.append(&mut doc.categories());
                }
                println!("{}", info.label());
            },
            Err(e) => eprintln!("{}: {}", info.file.path.display(), e),
        }
    }
}

pub fn extract_metadata_from_filename(metadata: &mut Metadata) {
    for info in metadata {
        if !info.title.is_empty() {
            continue;
        }

        if let Some(filename) = info.file.path.file_name().and_then(OsStr::to_str) {
            let mut start_index = 0;

            if filename.starts_with('(') {
                start_index += 1;
                if let Some(index) = filename[start_index..].find(')') {
                    info.series = filename[start_index..start_index+index].trim_end().to_string();
                    start_index += index + 1;
                }
            }

            if let Some(index) = filename[start_index..].find("- ") {
                info.author = filename[start_index..start_index+index].trim().to_string();
                start_index += index + 1;
            }

            let title_start = start_index;

            if let Some(index) = filename[start_index..].find('_') {
                info.title = filename[start_index..start_index+index].trim_start().to_string();
                start_index += index + 1;
            }

            if let Some(index) = filename[start_index..].find('-') {
                if title_start == start_index {
                    info.title = filename[start_index..start_index+index].trim_start().to_string();
                } else {
                    info.subtitle = filename[start_index..start_index+index].trim_start().to_string();
                }
                start_index += index + 1;
            }

            if let Some(index) = filename[start_index..].find('(') {
                info.publisher = filename[start_index..start_index+index].trim_end().to_string();
                start_index += index + 1;
            }

            if let Some(index) = filename[start_index..].find(')') {
                info.year = filename[start_index..start_index+index].to_string();
            }

            println!("{}", info.label());
        }
    }
}

pub fn clean_up(dir: &Path, metadata: &mut Metadata) {
    metadata.retain(|info| {
        let path = &info.file.path;
        if !dir.join(path).exists() {
            println!("{}", path.display());
            false
        } else {
            true
        }
    });
}

fn find_files(root: &Path, dir: &Path, traverse_hidden: bool) -> Result<Vec<FileInfo>, Error> {
    let mut result = Vec::new();

    for entry in fs::read_dir(dir).context("Can't read directory.")? {
        let entry = entry.context("Can't read directory entry.")?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                if (!traverse_hidden && name.starts_with('.')) || RESERVED_DIRECTORIES.contains(name) {
                    continue;
                }
            }
            result.extend_from_slice(&find_files(root, path.as_path(), traverse_hidden)?);
        } else {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let relat = path.strip_prefix(root).unwrap().to_path_buf();
            let kind = file_kind(path).unwrap_or_default();
            let size = entry.metadata().map(|m| m.len()).unwrap_or_default();

            result.push(
                FileInfo {
                    path: relat,
                    kind,
                    size,
                }
            );
        }
    }

    Ok(result)
}
