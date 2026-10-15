use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::ImageFormat;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{State, WebviewWindow, ipc::IpcResponse};
use tauri_runtime_cef::CefRuntime;
use uuid::Uuid;

use super::Error;

const MAX_CATALOG_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PAGE_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 50_000;
const MAX_COVER_CACHE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReaderLayout {
    #[default]
    Single,
    Spread,
    Webtoon,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReaderDirection {
    #[default]
    Rtl,
    Ltr,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReaderFit {
    #[default]
    Page,
    Width,
    Height,
    Original,
}

#[derive(Clone, Debug, Deserialize, Serialize, Type)]
pub struct ReaderSettings {
    pub layout: ReaderLayout,
    pub direction: ReaderDirection,
    pub fit: ReaderFit,
    pub zoom: f64,
}

impl Default for ReaderSettings {
    fn default() -> Self {
        Self {
            layout: ReaderLayout::Single,
            direction: ReaderDirection::Rtl,
            fit: ReaderFit::Page,
            zoom: 1.0,
        }
    }
}

impl ReaderSettings {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.zoom.is_finite() && (0.1..=8.0).contains(&self.zoom),
            "reader zoom must be between 10% and 800%"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Type)]
pub struct ReaderProgress {
    #[specta(type = f64)]
    pub page: usize,
    pub offset: f64,
    pub finished: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct ReaderBook {
    pub id: String,
    pub path: String,
    pub name: String,
    pub folder: String,
    #[specta(type = f64)]
    pub pages: usize,
    #[specta(type = f64)]
    pub size: u64,
    pub added_at: f64,
    pub thumbnail: Option<String>,
    pub error: Option<String>,
    pub progress: Option<ReaderProgress>,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct ReaderFolderError {
    pub path: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct ReaderLibrary {
    pub folders: Vec<String>,
    pub books: Vec<ReaderBook>,
    pub settings: ReaderSettings,
    pub folder_errors: Vec<ReaderFolderError>,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct ReaderPageInfo {
    pub name: String,
    pub mime: String,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct ReaderSession {
    pub id: String,
    pub name: String,
    pub pages: Vec<ReaderPageInfo>,
    #[specta(type = f64)]
    pub page: usize,
}

#[derive(Type)]
#[specta(transparent)]
pub(crate) struct ReaderBytes(#[specta(type = Vec<u8>)] Vec<u8>);

impl IpcResponse for ReaderBytes {
    fn body(self) -> tauri::Result<tauri::ipc::InvokeResponseBody> {
        Ok(self.0.into())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct FileVersion {
    size: u64,
    modified: Option<(u64, u32)>,
}

impl FileVersion {
    fn of(file: &File) -> Result<Self> {
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "reader archive is not a file");
        Ok(Self {
            size: metadata.len(),
            modified: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|time| (time.as_secs(), time.subsec_nanos())),
        })
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct CatalogBook {
    id: String,
    path: PathBuf,
    name: String,
    folder: PathBuf,
    pages: usize,
    size: u64,
    added_at: f64,
    version: Option<FileVersion>,
    progress: Option<ReaderProgress>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct Catalog {
    folders: Vec<PathBuf>,
    books: Vec<CatalogBook>,
    settings: ReaderSettings,
}

#[derive(Clone)]
struct PageEntry {
    index: usize,
    name: String,
    mime: &'static str,
    size: u64,
    crc: u32,
}

struct OpenBook {
    id: String,
    path: PathBuf,
    version: FileVersion,
    pages: Vec<PageEntry>,
    cancelled: AtomicBool,
}

impl OpenBook {
    fn read(&self, page: usize) -> Result<Vec<u8>> {
        ensure!(
            !self.cancelled.load(Ordering::Acquire),
            "reader session is closed"
        );
        let entry = self
            .pages
            .get(page)
            .context("page is outside this chapter")?;
        let file = File::open(&self.path).context("could not open the chapter file")?;
        ensure!(
            FileVersion::of(&file)? == self.version,
            "chapter changed on disk; reopen it to load the updated archive"
        );
        let mut archive = zip::ZipArchive::new(file)?;
        let bytes = read_entry(&mut archive, entry)?;
        let file = archive.into_inner();
        ensure!(
            FileVersion::of(&file)? == self.version
                && FileVersion::of(&File::open(&self.path)?)? == self.version,
            "chapter changed while reading; reopen it to load the updated archive"
        );
        ensure!(
            !self.cancelled.load(Ordering::Acquire),
            "reader session is closed"
        );
        Ok(bytes)
    }
}

struct Cover {
    version: FileVersion,
    data: String,
}

struct LibraryState {
    path: PathBuf,
    load_error: Option<String>,
    catalog: Catalog,
    errors: BTreeMap<String, String>,
    folder_errors: Vec<ReaderFolderError>,
    covers: BTreeMap<String, Cover>,
    session: Option<Arc<OpenBook>>,
}

pub(crate) struct ReaderState {
    inner: Arc<Mutex<LibraryState>>,
    reads: Arc<tokio::sync::Semaphore>,
}

impl ReaderState {
    pub(crate) fn new() -> Result<Self> {
        let path = dirs::data_local_dir()
            .context("could not locate the local application data directory")?
            .join("koharu")
            .join("reader-library.json");
        match Self::at(path.clone()) {
            Ok(state) => Ok(state),
            Err(error) => Ok(Self {
                reads: Arc::new(tokio::sync::Semaphore::new(2)),
                inner: Arc::new(Mutex::new(LibraryState {
                    path,
                    load_error: Some(format!(
                        "Saved reader library could not be opened: {error:#}. The file was preserved."
                    )),
                    catalog: Catalog::default(),
                    errors: BTreeMap::new(),
                    folder_errors: Vec::new(),
                    covers: BTreeMap::new(),
                    session: None,
                })),
            }),
        }
    }

    fn at(path: PathBuf) -> Result<Self> {
        let catalog = match File::open(&path) {
            Ok(file) => {
                ensure!(
                    file.metadata()?.len() <= MAX_CATALOG_BYTES,
                    "reader library is too large"
                );
                let catalog: Catalog = serde_json::from_reader(file)
                    .context("could not read the saved reader library")?;
                catalog.settings.validate()?;
                catalog
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Catalog::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            reads: Arc::new(tokio::sync::Semaphore::new(2)),
            inner: Arc::new(Mutex::new(LibraryState {
                path,
                load_error: None,
                catalog,
                errors: BTreeMap::new(),
                folder_errors: Vec::new(),
                covers: BTreeMap::new(),
                session: None,
            })),
        })
    }
}

impl LibraryState {
    fn persist(&self, catalog: &Catalog) -> Result<()> {
        if let Some(error) = &self.load_error {
            bail!("{error}");
        }
        let parent = self
            .path
            .parent()
            .context("reader library has no parent folder")?;
        fs::create_dir_all(parent)?;
        let bytes = serde_json::to_vec(catalog)?;
        ensure!(
            bytes.len() as u64 <= MAX_CATALOG_BYTES,
            "reader library is too large"
        );
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|error| error.error)?;
        Ok(())
    }

    fn library(&self) -> ReaderLibrary {
        ReaderLibrary {
            folders: self
                .catalog
                .folders
                .iter()
                .map(|path| display_path(path))
                .collect(),
            settings: self.catalog.settings.clone(),
            folder_errors: self.folder_errors.clone(),
            books: self
                .catalog
                .books
                .iter()
                .map(|book| ReaderBook {
                    id: book.id.clone(),
                    path: display_path(&book.path),
                    name: book.name.clone(),
                    folder: display_path(&book.folder),
                    pages: book.pages,
                    size: book.size,
                    added_at: book.added_at,
                    thumbnail: self.covers.get(&book.id).map(|cover| cover.data.clone()),
                    error: self.errors.get(&book.id).cloned(),
                    progress: book.progress.clone(),
                })
                .collect(),
        }
    }

    fn refresh(&mut self, folder: Option<PathBuf>) -> Result<ReaderLibrary> {
        if let Some(error) = &self.load_error {
            bail!("{error}");
        }
        let mut catalog = self.catalog.clone();
        if let Some(folder) = folder {
            let folder =
                fs::canonicalize(folder).context("could not access the selected folder")?;
            ensure!(folder.is_dir(), "select a folder containing CBZ chapters");
            if !catalog.folders.contains(&folder) {
                catalog.folders.push(folder);
            }
        }
        let mut paths = BTreeSet::new();
        let mut folder_errors = Vec::new();
        let mut owners = BTreeMap::new();
        for folder in &catalog.folders {
            let entries = match fs::read_dir(folder) {
                Ok(entries) => entries,
                Err(error) => {
                    folder_errors.push(ReaderFolderError {
                        path: display_path(folder),
                        error: format!("Folder unavailable: {error}"),
                    });
                    continue;
                }
            };
            for entry in entries {
                match entry {
                    Ok(entry)
                        if entry.file_type().is_ok_and(|kind| kind.is_file())
                            && entry
                                .path()
                                .extension()
                                .is_some_and(|extension| extension.eq_ignore_ascii_case("cbz")) =>
                    {
                        match fs::canonicalize(entry.path()) {
                            Ok(path) => {
                                owners.entry(path.clone()).or_insert_with(|| folder.clone());
                                paths.insert(path);
                            }
                            Err(error) => folder_errors.push(ReaderFolderError {
                                path: display_path(folder),
                                error: format!(
                                    "Could not inspect {}: {error}",
                                    entry.path().display()
                                ),
                            }),
                        }
                    }
                    Ok(_) => {}
                    Err(error) => folder_errors.push(ReaderFolderError {
                        path: display_path(folder),
                        error: format!("Could not scan a folder entry: {error}"),
                    }),
                }
            }
        }
        let mut paths = paths.into_iter().collect::<Vec<_>>();
        alphanumeric_sort::sort_slice_by_path_key(&mut paths, PathBuf::as_path);
        let mut errors = BTreeMap::new();
        let mut remaining_cover_bytes = MAX_COVER_CACHE_BYTES;
        let mut covers = BTreeMap::new();
        for path in &paths {
            let index = catalog.books.iter().position(|book| &book.path == path);
            let index = index.unwrap_or_else(|| {
                catalog.books.push(CatalogBook {
                    id: Uuid::new_v4().to_string(),
                    path: path.clone(),
                    name: path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    folder: owners[path].clone(),
                    pages: 0,
                    size: 0,
                    added_at: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs_f64()
                        * 1000.0,
                    version: None,
                    progress: None,
                });
                catalog.books.len() - 1
            });
            let book = &mut catalog.books[index];
            let result = (|| -> Result<()> {
                let file = File::open(path)?;
                let version = FileVersion::of(&file)?;
                book.size = version.size;
                if let Some(cover) = self
                    .covers
                    .get(&book.id)
                    .filter(|cover| {
                        cover.version == version && book.version.as_ref() == Some(&version)
                    })
                    .filter(|cover| cover.data.len() <= remaining_cover_bytes)
                {
                    remaining_cover_bytes -= cover.data.len();
                    covers.insert(
                        book.id.clone(),
                        Cover {
                            version,
                            data: cover.data.clone(),
                        },
                    );
                    return Ok(());
                }
                let mut archive = zip::ZipArchive::new(file)?;
                let pages = index_pages(&mut archive)?;
                book.pages = pages.len();
                book.version = Some(version.clone());
                if let Some(progress) = &mut book.progress {
                    progress.page = progress.page.min(book.pages - 1);
                }
                if remaining_cover_bytes > 0 {
                    let bytes = read_entry(&mut archive, &pages[0])?;
                    let image = decode_cover(bytes)?.thumbnail(288, 384).to_rgba8();
                    let encoded =
                        webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height())
                            .encode(82.0);
                    let data = format!("data:image/webp;base64,{}", STANDARD.encode(&*encoded));
                    if data.len() <= remaining_cover_bytes {
                        remaining_cover_bytes -= data.len();
                        covers.insert(book.id.clone(), Cover { version, data });
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                errors.insert(book.id.clone(), format!("{error:#}"));
            }
        }
        for book in &catalog.books {
            if !paths.contains(&book.path) {
                errors.insert(
                    book.id.clone(),
                    "Chapter unavailable. Reconnect its folder or restore the file, then refresh."
                        .to_owned(),
                );
            }
        }
        self.persist(&catalog)?;
        self.catalog = catalog;
        self.errors = errors;
        self.folder_errors = folder_errors;
        self.covers = covers;
        Ok(self.library())
    }

    fn open(&mut self, book: &str) -> Result<ReaderSession> {
        let book = self
            .catalog
            .books
            .iter()
            .find(|entry| entry.id == book)
            .context("chapter is not registered in the library")?;
        let file = File::open(&book.path)
            .context("chapter is unavailable; reconnect its folder and refresh")?;
        let version = FileVersion::of(&file)?;
        let mut archive = zip::ZipArchive::new(file).context("could not open this CBZ chapter")?;
        let pages = index_pages(&mut archive)?;
        let id = Uuid::new_v4().to_string();
        let session = ReaderSession {
            id: id.clone(),
            name: book.name.clone(),
            page: book
                .progress
                .as_ref()
                .map_or(0, |progress| progress.page.min(pages.len() - 1)),
            pages: pages
                .iter()
                .map(|page| ReaderPageInfo {
                    name: page.name.clone(),
                    mime: page.mime.to_owned(),
                })
                .collect(),
        };
        let opened = Arc::new(OpenBook {
            id,
            path: book.path.clone(),
            version,
            pages,
            cancelled: AtomicBool::new(false),
        });
        if book.version.as_ref() != Some(&opened.version) || book.pages != opened.pages.len() {
            let mut catalog = self.catalog.clone();
            let updated = catalog
                .books
                .iter_mut()
                .find(|entry| entry.id == book.id)
                .context("chapter is no longer registered")?;
            updated.pages = opened.pages.len();
            updated.size = opened.version.size;
            updated.version = Some(opened.version.clone());
            if let Some(progress) = &mut updated.progress {
                progress.page = progress.page.min(updated.pages - 1);
            }
            self.persist(&catalog)?;
            self.catalog = catalog;
        }
        if let Some(previous) = self.session.replace(opened) {
            previous.cancelled.store(true, Ordering::Release);
        }
        Ok(session)
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn page_mime(name: &str) -> Option<&'static str> {
    let path = Path::new(name);
    if path
        .components()
        .any(|part| part.as_os_str().eq_ignore_ascii_case("__MACOSX"))
        || path.file_name()?.to_str()?.starts_with("._")
    {
        return None;
    }
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
        Some("image/jpeg")
    } else if extension.eq_ignore_ascii_case("png") {
        Some("image/png")
    } else if extension.eq_ignore_ascii_case("webp") {
        Some("image/webp")
    } else {
        None
    }
}

fn index_pages(archive: &mut zip::ZipArchive<File>) -> Result<Vec<PageEntry>> {
    ensure!(
        archive.len() <= MAX_ARCHIVE_ENTRIES,
        "archive contains too many entries"
    );
    let mut pages = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if !entry.is_file() {
            continue;
        }
        let Some(mime) = page_mime(entry.name()) else {
            continue;
        };
        ensure!(
            entry.size() <= MAX_PAGE_BYTES && entry.compressed_size() <= MAX_PAGE_BYTES,
            "page {} exceeds the 64 MiB reader limit",
            entry.name()
        );
        pages.push(PageEntry {
            index,
            name: entry.name().to_owned(),
            mime,
            size: entry.size(),
            crc: entry.crc32(),
        });
    }
    alphanumeric_sort::sort_slice_by_str_key(&mut pages, |page| &page.name);
    ensure!(
        !pages.is_empty(),
        "CBZ contains no supported JPEG, PNG, or WebP pages"
    );
    Ok(pages)
}

fn read_entry(archive: &mut zip::ZipArchive<File>, expected: &PageEntry) -> Result<Vec<u8>> {
    let mut entry = archive.by_index(expected.index)?;
    ensure!(
        entry.name() == expected.name
            && entry.size() == expected.size
            && entry.crc32() == expected.crc,
        "chapter contents changed; reopen it to load the updated archive"
    );
    let mut bytes = Vec::new();
    entry
        .by_ref()
        .take(MAX_PAGE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_PAGE_BYTES,
        "page exceeds the 64 MiB reader limit"
    );
    validate_image(&bytes, expected.mime)?;
    Ok(bytes)
}

fn validate_image(bytes: &[u8], expected_mime: &str) -> Result<ImageFormat> {
    let format =
        image::guess_format(bytes).context("page image format is unsupported or corrupt")?;
    let mime = match format {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Png => "image/png",
        ImageFormat::WebP => "image/webp",
        _ => bail!("page image format is unsupported"),
    };
    ensure!(
        mime == expected_mime,
        "page image format does not match its filename"
    );
    let (width, height) =
        image::ImageReader::with_format(Cursor::new(bytes), format).into_dimensions()?;
    ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PAGE_PIXELS,
        "page exceeds the 64 megapixel reader limit"
    );
    Ok(format)
}

fn decode_cover(bytes: Vec<u8>) -> Result<image::DynamicImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PAGE_PIXELS * 4);
    reader.limits(limits);
    Ok(reader.decode()?)
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> std::result::Result<T, Error> {
    Ok(tokio::task::spawn_blocking(operation)
        .await
        .context("reader operation stopped unexpectedly")??)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn get_reader_library(
    state: State<'_, ReaderState>,
) -> std::result::Result<ReaderLibrary, Error> {
    let state = state.inner.clone();
    blocking(move || state.lock().refresh(None)).await
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn browse_reader_folder(
    window: WebviewWindow<CefRuntime>,
    state: State<'_, ReaderState>,
) -> std::result::Result<Option<ReaderLibrary>, Error> {
    let Some(folder) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .pick_folder()
        .await
    else {
        return Ok(None);
    };
    let path = folder.path().to_owned();
    let state = state.inner.clone();
    blocking(move || state.lock().refresh(Some(path)).map(Some)).await
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn refresh_reader_library(
    state: State<'_, ReaderState>,
) -> std::result::Result<ReaderLibrary, Error> {
    let state = state.inner.clone();
    blocking(move || state.lock().refresh(None)).await
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn open_reader_book(
    book: String,
    state: State<'_, ReaderState>,
) -> std::result::Result<ReaderSession, Error> {
    let state = state.inner.clone();
    blocking(move || state.lock().open(&book)).await
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn get_reader_page(
    session: String,
    page: u32,
    state: State<'_, ReaderState>,
) -> std::result::Result<ReaderBytes, Error> {
    let permit = state
        .reads
        .clone()
        .acquire_owned()
        .await
        .context("reader page loading has stopped")?;
    let state = state.inner.clone();
    blocking(move || {
        let _permit = permit;
        let opened = state
            .lock()
            .session
            .as_ref()
            .filter(|opened| opened.id == session)
            .cloned()
            .context("reader session is closed or no longer current")?;
        opened.read(page as usize).map(ReaderBytes)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn close_reader_book(
    session: String,
    state: State<'_, ReaderState>,
) -> std::result::Result<(), Error> {
    let state = state.inner.clone();
    blocking(move || {
        let mut state = state.lock();
        if state
            .session
            .as_ref()
            .is_some_and(|opened| opened.id == session)
            && let Some(opened) = state.session.take()
        {
            opened.cancelled.store(true, Ordering::Release);
        }
        Ok(())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn save_reader_settings(
    settings: ReaderSettings,
    state: State<'_, ReaderState>,
) -> std::result::Result<ReaderSettings, Error> {
    settings.validate()?;
    let state = state.inner.clone();
    blocking(move || {
        let mut state = state.lock();
        let mut catalog = state.catalog.clone();
        catalog.settings = settings.clone();
        state.persist(&catalog)?;
        state.catalog = catalog;
        Ok(settings)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn save_reader_progress(
    book: String,
    page: u32,
    offset: f64,
    finished: bool,
    state: State<'_, ReaderState>,
) -> std::result::Result<(), Error> {
    let page = page as usize;
    let state = state.inner.clone();
    blocking(move || {
        ensure!(
            offset.is_finite() && (0.0..=1.0).contains(&offset),
            "reading offset must be between 0 and 1"
        );
        let mut state = state.lock();
        let mut catalog = state.catalog.clone();
        let book = catalog
            .books
            .iter_mut()
            .find(|entry| entry.id == book)
            .context("chapter is not registered in the library")?;
        ensure!(
            page < book.pages,
            "reading position is outside this chapter"
        );
        book.progress = Some(ReaderProgress {
            page,
            offset,
            finished,
        });
        state.persist(&catalog)?;
        state.catalog = catalog;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::new(3, 4)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn archive(path: &Path, entries: &[(&str, &[u8])]) {
        let mut archive = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, bytes) in entries {
            archive
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap();
    }

    #[test]
    fn opens_natural_page_order_and_only_reads_the_requested_member() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chapter.cbz");
        let image = png();
        archive(
            &path,
            &[
                ("page10.png", b"corrupt"),
                ("__MACOSX/._page.png", &image),
                ("page2.png", &image),
                ("ComicInfo.xml", b"metadata"),
            ],
        );
        let state = ReaderState::at(directory.path().join("catalog.json")).unwrap();
        let mut state = state.inner.lock();
        let library = state.refresh(Some(directory.path().to_owned())).unwrap();
        assert_eq!(library.books[0].pages, 2);
        let opened = state.open(&library.books[0].id).unwrap();
        assert_eq!(
            opened
                .pages
                .iter()
                .map(|page| page.name.as_str())
                .collect::<Vec<_>>(),
            ["page2.png", "page10.png"]
        );
        let session = state.session.as_ref().unwrap();
        assert_eq!(session.read(0).unwrap(), image);
        assert!(session.read(1).unwrap_err().to_string().contains("format"));
        assert!(session.read(2).is_err());
    }

    #[test]
    fn refresh_preserves_identity_progress_and_unavailable_folder_history() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().join("books");
        fs::create_dir(&folder).unwrap();
        archive(&folder.join("chapter.cbz"), &[("page.png", &png())]);
        fs::write(folder.join("broken.cbz"), b"not a zip").unwrap();
        let catalog = directory.path().join("catalog.json");
        let reader = ReaderState::at(catalog.clone()).unwrap();
        {
            let mut state = reader.inner.lock();
            let library = state.refresh(Some(folder.clone())).unwrap();
            assert_eq!(library.books.len(), 2);
            assert!(
                library
                    .books
                    .iter()
                    .find(|book| book.name == "broken")
                    .unwrap()
                    .error
                    .is_some()
            );
            let book = state
                .catalog
                .books
                .iter_mut()
                .find(|book| book.name == "chapter")
                .unwrap();
            book.progress = Some(ReaderProgress {
                page: 0,
                offset: 0.4,
                finished: false,
            });
            state.catalog.settings.zoom = 1.5;
            state.persist(&state.catalog).unwrap();
        }
        fs::rename(&folder, directory.path().join("disconnected")).unwrap();
        let reloaded = ReaderState::at(catalog).unwrap();
        let library = reloaded.inner.lock().refresh(None).unwrap();
        let chapter = library
            .books
            .iter()
            .find(|book| book.name == "chapter")
            .unwrap();
        assert_eq!(chapter.progress.as_ref().unwrap().offset, 0.4);
        assert_eq!(library.settings.zoom, 1.5);
        assert!(chapter.error.is_some());
        assert_eq!(library.folder_errors.len(), 1);
    }

    #[test]
    fn rejects_closed_sessions_and_changed_archive_versions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chapter.cbz");
        archive(&path, &[("page.png", &png())]);
        let reader = ReaderState::at(directory.path().join("catalog.json")).unwrap();
        let mut state = reader.inner.lock();
        let library = state.refresh(Some(directory.path().to_owned())).unwrap();
        state.open(&library.books[0].id).unwrap();
        let previous = state.session.as_ref().unwrap().clone();
        state.open(&library.books[0].id).unwrap();
        assert!(previous.read(0).unwrap_err().to_string().contains("closed"));
        let current = state.session.as_ref().unwrap().clone();
        archive(&path, &[("page.png", &png()), ("page2.png", &png())]);
        assert!(current.read(0).unwrap_err().to_string().contains("changed"));
    }

    #[test]
    fn settings_and_image_headers_are_validated_before_use() {
        let mut settings = ReaderSettings::default();
        assert!(settings.validate().is_ok());
        settings.zoom = f64::NAN;
        assert!(settings.validate().is_err());
        settings.zoom = 9.0;
        assert!(settings.validate().is_err());
        assert!(validate_image(&png(), "image/jpeg").is_err());
        assert!(validate_image(b"not an image", "image/png").is_err());
    }
}
