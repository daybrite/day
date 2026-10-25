// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Imperative, awaitable presentations: the `alert` / `confirm` / `prompt` dialogs and the
//! `open_file` / `open_files` / `open_folder` / `save_file` system file pickers (`FileUrl`).
//! Each returns a future you await for the user's choice.

use std::hash::Hash;

use crate::*;

// ---------------------------------------------------------------------------
// Imperative presentation (docs/dialogs.md)
// ---------------------------------------------------------------------------

use std::future::{Future, IntoFuture};
use std::pin::Pin;

use day_spec::present::{ButtonRole, PresentButton, PresentResult, PresentSpec};

/// Boxed future the awaitable presenters resolve to: one alloc per dialog, negligible.
type Presenting<T> = Pin<Box<dyn Future<Output = T>>>;

/// A dialog / confirmation / action sheet. Buttons carry a typed payload `T`; `.present()`
/// awaits and returns the chosen button's payload, or `None` on cancel/dismiss.
///
/// ```ignore
/// let choice = Alert::new(tr("delete-title"))
///     .message(tr("delete-body"))
///     .destructive(tr("delete"), Choice::Delete)
///     .cancel(tr("cancel"))
///     .present().await;   // Option<Choice>
/// ```
pub struct Alert<T> {
    title: String,
    message: Option<String>,
    sheet: bool,
    /// (label, role, payload) in presentation order; cancel buttons carry `None`.
    buttons: Vec<(String, ButtonRole, Option<T>)>,
}

pub fn alert<M>(title: impl IntoText<M>) -> Alert<()> {
    Alert {
        title: title.into_text().initial(),
        message: None,
        sheet: false,
        buttons: Vec::new(),
    }
}

impl<T> Alert<T> {
    pub fn new<M>(title: impl IntoText<M>) -> Alert<T> {
        Alert {
            title: title.into_text().initial(),
            message: None,
            sheet: false,
            buttons: Vec::new(),
        }
    }
    pub fn message<M>(mut self, m: impl IntoText<M>) -> Self {
        self.message = Some(m.into_text().initial());
        self
    }
    /// Present as a bottom action sheet on mobile (desktop falls back to an alert).
    pub fn sheet(mut self) -> Self {
        self.sheet = true;
        self
    }
    /// A normal choice carrying `value`.
    pub fn button<M>(mut self, label: impl IntoText<M>, value: T) -> Self {
        self.buttons.push((
            label.into_text().initial(),
            ButtonRole::Default,
            Some(value),
        ));
        self
    }
    /// A destructive choice (red on Apple) carrying `value`.
    pub fn destructive<M>(mut self, label: impl IntoText<M>, value: T) -> Self {
        self.buttons.push((
            label.into_text().initial(),
            ButtonRole::Destructive,
            Some(value),
        ));
        self
    }
    /// The cancel affordance; choosing it (or dismissing) resolves to `None`.
    pub fn cancel<M>(mut self, label: impl IntoText<M>) -> Self {
        self.buttons
            .push((label.into_text().initial(), ButtonRole::Cancel, None));
        self
    }

    /// Present natively and await the chosen payload (`None` = cancel / dismissed).
    pub async fn present(self) -> Option<T> {
        let spec = PresentSpec::Dialog {
            title: self.title,
            message: self.message,
            buttons: self
                .buttons
                .iter()
                .map(|(label, role, _)| PresentButton {
                    label: label.clone(),
                    role: *role,
                })
                .collect(),
            sheet: self.sheet,
        };
        let mut payloads: Vec<Option<T>> = self.buttons.into_iter().map(|(_, _, v)| v).collect();
        match day_core::present(spec).await {
            PresentResult::Button(i) => {
                let i = i as usize;
                if i < payloads.len() {
                    payloads[i].take()
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl<T: 'static> IntoFuture for Alert<T> {
    type Output = Option<T>;
    type IntoFuture = Presenting<Option<T>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

/// A yes/no confirmation. Resolves to `true` only if the confirm button is chosen.
pub struct Confirm {
    title: String,
    message: Option<String>,
    confirm: String,
    cancel: String,
    destructive: bool,
}

pub fn confirm<M>(title: impl IntoText<M>) -> Confirm {
    Confirm {
        title: title.into_text().initial(),
        message: None,
        // Localized from the core catalog (docs/dialogs.md); `.confirm_label`/`.cancel_label`
        // override. Resolved in the current locale at build time.
        confirm: day_localization::t(day_localization::key::day_ok),
        cancel: day_localization::t(day_localization::key::day_cancel),
        destructive: false,
    }
}

impl Confirm {
    pub fn message<M>(mut self, m: impl IntoText<M>) -> Self {
        self.message = Some(m.into_text().initial());
        self
    }
    pub fn confirm_label<M>(mut self, label: impl IntoText<M>) -> Self {
        self.confirm = label.into_text().initial();
        self
    }
    pub fn cancel_label<M>(mut self, label: impl IntoText<M>) -> Self {
        self.cancel = label.into_text().initial();
        self
    }
    /// Style the confirm button as destructive.
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
    pub async fn present(self) -> bool {
        let confirm_role = if self.destructive {
            ButtonRole::Destructive
        } else {
            ButtonRole::Default
        };
        let spec = PresentSpec::Dialog {
            title: self.title,
            message: self.message,
            buttons: vec![
                PresentButton {
                    label: self.cancel,
                    role: ButtonRole::Cancel,
                },
                PresentButton {
                    label: self.confirm,
                    role: confirm_role,
                },
            ],
            sheet: false,
        };
        // index 1 = the confirm button.
        matches!(day_core::present(spec).await, PresentResult::Button(1))
    }
}

impl IntoFuture for Confirm {
    type Output = bool;
    type IntoFuture = Presenting<bool>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

/// A single-line text prompt. Resolves to `Some(text)` on OK, `None` on cancel/dismiss.
pub struct Prompt {
    title: String,
    message: Option<String>,
    placeholder: String,
    initial: String,
    ok: String,
    cancel: String,
}

pub fn prompt<M>(title: impl IntoText<M>) -> Prompt {
    Prompt {
        title: title.into_text().initial(),
        message: None,
        placeholder: String::new(),
        initial: String::new(),
        // Localized from the core catalog (docs/dialogs.md); `.ok_label`/`.cancel_label` override.
        ok: day_localization::t(day_localization::key::day_ok),
        cancel: day_localization::t(day_localization::key::day_cancel),
    }
}

impl Prompt {
    pub fn message<M>(mut self, m: impl IntoText<M>) -> Self {
        self.message = Some(m.into_text().initial());
        self
    }
    pub fn placeholder<M>(mut self, p: impl IntoText<M>) -> Self {
        self.placeholder = p.into_text().initial();
        self
    }
    pub fn initial<M>(mut self, v: impl IntoText<M>) -> Self {
        self.initial = v.into_text().initial();
        self
    }
    pub fn ok_label<M>(mut self, label: impl IntoText<M>) -> Self {
        self.ok = label.into_text().initial();
        self
    }
    pub fn cancel_label<M>(mut self, label: impl IntoText<M>) -> Self {
        self.cancel = label.into_text().initial();
        self
    }
    pub async fn present(self) -> Option<String> {
        let spec = PresentSpec::Prompt {
            title: self.title,
            message: self.message,
            placeholder: self.placeholder,
            initial: self.initial,
            ok: self.ok,
            cancel: self.cancel,
        };
        match day_core::present(spec).await {
            PresentResult::Text(t) => Some(t),
            _ => None,
        }
    }
}

impl IntoFuture for Prompt {
    type Output = Option<String>;
    type IntoFuture = Presenting<Option<String>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

// ---------------------------------------------------------------------------
// File open / save (docs/files.md)
// ---------------------------------------------------------------------------

use day_spec::present::FileFilter;

/// A cross-platform handle to a file the user chose in a native open/save picker.
///
/// Internally a single **locator string**. On desktop and iOS it is an absolute filesystem
/// path; on Android it may be a `content://` URI, since the Storage Access Framework does not
/// expose real filesystem paths. That is why Day uses a bespoke type rather than
/// [`std::path::PathBuf`] (which cannot represent a `content://` URI) or a bare `String` (no
/// type-safety / helpers): a `FileUrl` is the lossless union with ergonomic accessors.
///
/// Files returned from [`open_file`] are always readable via [`FileUrl::read_to_string`] /
/// [`FileUrl::read`]: backends copy a picked file into app storage first where the platform
/// requires it, so the local path works everywhere.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileUrl(String);

impl FileUrl {
    /// Wrap a locator string (a filesystem path or a URI). Usually produced by the pickers.
    pub fn new(locator: impl Into<String>) -> Self {
        FileUrl(locator.into())
    }
    /// The raw locator: a filesystem path, or a `content://`-style URI on Android.
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// The locator as a filesystem path: `Some` for local paths (and `file://` URLs), `None`
    /// for opaque URIs such as Android's `content://`.
    pub fn local_path(&self) -> Option<std::path::PathBuf> {
        if self.0.starts_with("file://") {
            return day_spec::transfer::file_paths(self.0.as_bytes())?
                .into_iter()
                .next();
        }
        if self.0.contains("://") {
            return None;
        }
        Some(std::path::PathBuf::from(&self.0))
    }
    /// Persistable macOS security-scoped bookmark for a file the user has authorized.
    /// Store the opaque bytes in app-private storage, not the path. Other platforms return
    /// `Unsupported`. `read_only` prevents a bookmark from requesting write access.
    pub fn bookmark(&self, read_only: bool) -> std::io::Result<Vec<u8>> {
        crate::file_access::bookmark(self, read_only)
    }
    /// Resolve a saved macOS bookmark and hold its security scope until the returned guard
    /// is dropped. Keep the guard alive across all I/O, including asynchronous operations.
    pub fn resolve_bookmark(bytes: &[u8]) -> std::io::Result<crate::FileAccess> {
        crate::file_access::resolve(bytes)
    }
    /// The last path component, for display (e.g. `notes.txt`). Best-effort for opaque URIs.
    pub fn file_name(&self) -> Option<String> {
        let s = self.0.trim_end_matches(['/', '\\']);
        let tail = s.rsplit(['/', '\\']).next().unwrap_or(s);
        if tail.is_empty() {
            None
        } else {
            Some(tail.to_string())
        }
    }
    /// Read the file's bytes. Local paths only; opaque URIs return an `Unsupported` error.
    pub fn read(&self) -> std::io::Result<Vec<u8>> {
        match self.local_path() {
            Some(p) => fs_read(&p),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "content:// URIs are not directly readable",
            )),
        }
    }
    /// Read at most `limit` bytes without blocking the UI. Native reads run on a worker;
    /// browser-picked bytes are already in memory. Oversized input returns `InvalidData`.
    /// Apple security-scoped access is held for the read and released on every exit path.
    pub async fn read_limited(&self, limit: usize) -> std::io::Result<Vec<u8>> {
        #[cfg(target_arch = "wasm32")]
        {
            let bytes = self.read()?;
            if bytes.len() > limit {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            Ok(bytes)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // A job on the blocking pool (docs/async.md): a panicking or never-run job is an
            // I/O error here, like a worker that failed to start was.
            let file = self.clone();
            match day_core::spawn_blocking(move || file.read_bounded(limit)).await {
                Ok(result) => result,
                Err(e) => Err(std::io::Error::other(e)),
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn read_bounded(&self, limit: usize) -> std::io::Result<Vec<u8>> {
        use std::io::Read;
        let path = self.local_path().ok_or(std::io::ErrorKind::Unsupported)?;
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let _access = crate::file_access::scoped_read(&path)?;
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > limit as u64 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let mut bytes = Vec::new();
        file.take((limit as u64).saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        Ok(bytes)
    }
    /// Read the file as UTF-8 text.
    pub fn read_to_string(&self) -> std::io::Result<String> {
        match self.local_path() {
            Some(p) => fs_read(&p).and_then(|b| {
                String::from_utf8(b).map_err(|_| std::io::ErrorKind::InvalidData.into())
            }),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "content:// URIs are not directly readable",
            )),
        }
    }
}

impl std::fmt::Display for FileUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn parse_filter(name: impl Into<String>, extensions: &[&str]) -> FileFilter {
    FileFilter {
        name: name.into(),
        extensions: extensions
            .iter()
            .map(|e| e.trim_start_matches('.').to_string())
            .collect(),
    }
}

/// A native "open file" picker. `.await` (or `.present().await`) resolves to the chosen
/// [`FileUrl`], or `None` if the user cancels.
///
/// ```ignore
/// let file = open_file().filter("Text", &["txt", "md"]).await;
/// if let Some(f) = file { let body = f.read_to_string()?; }
/// ```
pub struct OpenFile {
    title: String,
    filters: Vec<FileFilter>,
}

/// Start a native open-file picker (docs/files.md).
pub fn open_file() -> OpenFile {
    OpenFile {
        title: day_localization::t(day_localization::key::day_open),
        filters: Vec::new(),
    }
}

impl OpenFile {
    /// Override the picker's title (localizable via `tr()`).
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text().initial();
        self
    }
    /// Add a named file-type filter, e.g. `.filter("Text", &["txt", "md"])`. No filter = all files.
    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push(parse_filter(name, extensions));
        self
    }
    pub async fn present(self) -> Option<FileUrl> {
        let spec = PresentSpec::OpenFile {
            title: self.title,
            filters: self.filters,
            multiple: false,
        };
        match day_core::present(spec).await {
            PresentResult::Files(mut v) if !v.is_empty() => Some(FileUrl(v.remove(0))),
            _ => None,
        }
    }
}

impl IntoFuture for OpenFile {
    type Output = Option<FileUrl>;
    type IntoFuture = Presenting<Option<FileUrl>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

/// A native "open files" picker that lets the user choose several files at once. `.await`
/// (or `.present().await`) resolves to the chosen [`FileUrl`]s in the picker's order, or an
/// empty `Vec` if the user cancels.
///
/// ```ignore
/// let tracks = open_files().filter("Audio", &["mp3", "flac"]).await;
/// for track in &tracks { library.add(track.read()?); }
/// ```
pub struct OpenFiles {
    title: String,
    filters: Vec<FileFilter>,
}

/// Start a native multi-file open picker (docs/files.md).
pub fn open_files() -> OpenFiles {
    OpenFiles {
        title: day_localization::t(day_localization::key::day_open),
        filters: Vec::new(),
    }
}

impl OpenFiles {
    /// Override the picker's title (localizable via `tr()`).
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text().initial();
        self
    }
    /// Add a named file-type filter, e.g. `.filter("Audio", &["mp3", "flac"])`. No filter = all files.
    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push(parse_filter(name, extensions));
        self
    }
    pub async fn present(self) -> Vec<FileUrl> {
        let spec = PresentSpec::OpenFile {
            title: self.title,
            filters: self.filters,
            multiple: true,
        };
        match day_core::present(spec).await {
            PresentResult::Files(v) => v.into_iter().map(FileUrl).collect(),
            _ => Vec::new(),
        }
    }
}

impl IntoFuture for OpenFiles {
    type Output = Vec<FileUrl>;
    type IntoFuture = Presenting<Vec<FileUrl>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

/// A native "choose folder" picker. `.await` (or `.present().await`) resolves to the chosen
/// directory as a [`FileUrl`], or `None` if the user cancels — and `None` at once on a toolkit
/// without a folder chooser, so ask [`Cap::FolderDialog`](day_spec::Cap::FolderDialog) before
/// offering the command (phones answer `Unsupported`).
///
/// ```ignore
/// if let Some(dir) = open_folder().title(res::str::add_folder()).await {
///     scan(dir.local_path()?);
/// }
/// ```
pub struct OpenFolder {
    title: String,
}

/// Start a native folder chooser (docs/files.md).
pub fn open_folder() -> OpenFolder {
    OpenFolder {
        title: day_localization::t(day_localization::key::day_open_folder),
    }
}

impl OpenFolder {
    /// Override the picker's title (localizable via `tr()`).
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text().initial();
        self
    }
    pub async fn present(self) -> Option<FileUrl> {
        let spec = PresentSpec::OpenFolder { title: self.title };
        match day_core::present(spec).await {
            PresentResult::Files(mut v) if !v.is_empty() => Some(FileUrl(v.remove(0))),
            _ => None,
        }
    }
}

impl IntoFuture for OpenFolder {
    type Output = Option<FileUrl>;
    type IntoFuture = Presenting<Option<FileUrl>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

/// A native "save file" picker carrying the bytes to write. `.await` resolves to the chosen
/// destination [`FileUrl`], or `None` on cancel.
///
/// ```ignore
/// let saved = save_file(text.into_bytes())
///     .suggested_name("notes.txt")
///     .filter("Text", &["txt"])
///     .await;
/// ```
pub struct SaveFile {
    title: String,
    suggested_name: String,
    filters: Vec<FileFilter>,
    data: Vec<u8>,
}

/// Start a native save-file picker for `data` (docs/files.md).
pub fn save_file(data: impl Into<Vec<u8>>) -> SaveFile {
    SaveFile {
        title: day_localization::t(day_localization::key::day_save),
        suggested_name: "untitled.txt".to_string(),
        filters: Vec::new(),
        data: data.into(),
    }
}

impl SaveFile {
    /// Override the picker's title (localizable via `tr()`).
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text().initial();
        self
    }
    /// The default file name shown in the picker.
    pub fn suggested_name(mut self, name: impl Into<String>) -> Self {
        self.suggested_name = name.into();
        self
    }
    /// Add a named file-type filter, e.g. `.filter("Text", &["txt"])`.
    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push(parse_filter(name, extensions));
        self
    }
    pub async fn present(self) -> Option<FileUrl> {
        // Stage the bytes in an app-writable temp file the backend hands to the native picker.
        let mut src = day_core::app_temp_dir();
        src.push(format!(
            "day-save-{}-{}",
            stage_tag(),
            sanitize_name(&self.suggested_name)
        ));
        if !fs_write(&src, &self.data) {
            return None;
        }
        let spec = PresentSpec::SaveFile {
            title: self.title,
            suggested_name: self.suggested_name,
            src_path: src.to_string_lossy().into_owned(),
            filters: self.filters,
        };
        let dest = match day_core::present(spec).await {
            PresentResult::Files(mut v) if !v.is_empty() => FileUrl(v.remove(0)),
            _ => {
                fs_remove(&src);
                return None;
            }
        };
        // Best-effort deliver the bytes to a local destination (desktop, and headless dayscript).
        // On Android the destination is a `content://` URI the backend already wrote (no
        // `local_path`, so the copy is skipped); iOS delivers via the document exporter.
        if let Some(p) = dest.local_path()
            && p != src
        {
            fs_copy(&src, &p);
        }
        fs_remove(&src);
        Some(dest)
    }
}

impl IntoFuture for SaveFile {
    type Output = Option<FileUrl>;
    type IntoFuture = Presenting<Option<FileUrl>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

// The file flows' filesystem, behind four verbs: a browser has no `std::fs`, so the wasm arms
// go through day-spec's `web_files` byte store instead (day-toolkit-dom's picker fills it and the
// shim's download drains it), while every other platform reads and writes files on disk.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod files {
    use std::path::Path;
    pub(super) fn fs_read(p: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(p)
    }
    pub(super) fn fs_write(p: &Path, bytes: &[u8]) -> bool {
        std::fs::write(p, bytes).is_ok()
    }
    pub(super) fn fs_copy(src: &Path, dest: &Path) {
        let _ = std::fs::copy(src, dest);
    }
    pub(super) fn fs_remove(p: &Path) {
        let _ = std::fs::remove_file(p);
    }
    pub(super) fn stage_tag() -> u32 {
        std::process::id()
    }
}
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
mod files {
    use day_spec::present::web_files;
    use std::path::Path;
    pub(super) fn fs_read(p: &Path) -> std::io::Result<Vec<u8>> {
        web_files::read(&p.to_string_lossy())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no such web file"))
    }
    pub(super) fn fs_write(p: &Path, bytes: &[u8]) -> bool {
        web_files::write(&p.to_string_lossy(), bytes.to_vec());
        true
    }
    pub(super) fn fs_copy(src: &Path, dest: &Path) {
        if let Some(b) = web_files::read(&src.to_string_lossy()) {
            web_files::write(&dest.to_string_lossy(), b);
        }
    }
    pub(super) fn fs_remove(p: &Path) {
        web_files::remove(&p.to_string_lossy());
    }
    /// `std::process::id()` aborts on wasm; one page is one "process".
    pub(super) fn stage_tag() -> u32 {
        0
    }
}
use files::{fs_copy, fs_read, fs_remove, fs_write, stage_tag};

/// Keep a suggested file name safe as a temp-file component (path-separator / control-char free).
fn sanitize_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "untitled".to_string()
    } else {
        s
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod document_read_tests {
    use super::*;
    #[test]
    fn bounded_read_accepts_exact_limit_rejects_oversize_and_opaque_uris() {
        let path = std::env::temp_dir().join(format!("day-document-test-{}", std::process::id()));
        std::fs::write(&path, b"12345").unwrap();
        let file = FileUrl::new(path.to_str().unwrap());
        assert_eq!(file.read_bounded(5).unwrap(), b"12345");
        assert_eq!(
            file.read_bounded(4).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        assert_eq!(
            FileUrl::new("content://fixture/test")
                .read_bounded(5)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::Unsupported
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::conformance::{Case, Drive};
    use day_reactive::Signal;
    use day_spec::Cap;

    use crate::*;

    /// A page whose button asks `confirm` and shows the answer.
    fn asker() -> impl day_core::Piece {
        let answer = Signal::new(String::from("none"));
        column((
            label(move || format!("answer {}", answer.get())).id(day_core::id!(answer)),
            button("Ask")
                .action(move || {
                    let set = answer.setter();
                    day_core::task(async move {
                        let yes = confirm("Delete it?").present().await;
                        set.set(if yes { "yes" } else { "no" }.into());
                    });
                })
                .id(day_core::id!(ask)),
            // A focus target without an IME: opening the keyboard here would change the
            // viewport inherited by the next test on mobile toolkits.
            rectangle()
                .frame(40.0, 40.0)
                .focusable()
                .id(day_core::id!(after_dialog)),
        ))
    }

    /// A native confirmation presents with its title, and the button chosen comes back; the
    /// toolkit dismisses the dialog it showed.
    #[day_macros::test(day_core)]
    fn confirm_native_answer() -> Case {
        Case::new()
            .proves_cap(Cap::Dialogs)
            .proves_duty("present")
            .proves_duty("dismiss")
            .requires(Cap::Dialogs)
            .page(asker)
            .drive(|d: Drive| async move {
                d.dialogs_scripted(false).await?;
                d.tap("ask").await?;
                d.assert_presented(Some("Delete it?")).await?;
                d.respond(Some(1)).await?;
                d.assert_text("answer", "answer yes").await?;
                // A completed modal must release native focus to the underlying page,
                // including when a scripted answer arrives before its first frame.
                d.focus("after_dialog").await?;
                d.assert_focused("after_dialog", true).await
            })
    }

    /// Dismissing a native confirmation answers it as cancelled.
    #[day_macros::test(day_core)]
    fn confirm_native_dismiss() -> Case {
        Case::new()
            .proves_cap(Cap::Dialogs)
            .requires(Cap::Dialogs)
            .page(asker)
            .drive(|d: Drive| async move {
                d.dialogs_scripted(false).await?;
                d.tap("ask").await?;
                d.assert_presented(None).await?;
                d.respond(None).await?;
                d.assert_text("answer", "answer no").await?;
                d.focus("after_dialog").await?;
                d.assert_focused("after_dialog", true).await
            })
    }

    fn prompt_asker() -> impl day_core::Piece {
        let answer = Signal::new(String::from("pending"));
        column((
            label(move || answer.get()).id(day_core::id!(answer)),
            button("Ask").id(day_core::id!(ask)).action(move || {
                let set = answer.setter();
                day_core::task(async move {
                    let value = prompt("Your name")
                        .message("Enter a name")
                        .placeholder("Name")
                        .initial("Ada")
                        .present()
                        .await;
                    set.set(format!("{value:?}"));
                });
            }),
        ))
    }

    #[day_macros::test(day_core)]
    fn prompt_native_answer() -> Case {
        Case::new()
            .requires(Cap::Dialogs)
            .proves_cap(Cap::Dialogs)
            .proves_duty("present")
            .proves_duty("dismiss")
            .page(prompt_asker)
            .drive(|d: Drive| async move {
                d.dialogs_scripted(false).await?;
                d.tap("ask").await?;
                d.assert_presented(Some("Your name")).await?;
                d.respond_text("Zoë 東京\n\"quoted\"").await?;
                d.assert_text("answer", "Some(\"Zoë 東京\\n\\\"quoted\\\"\")")
                    .await?;
                d.tap("ask").await?;
                d.respond_text("").await?;
                d.assert_text("answer", "Some(\"\")").await
            })
    }

    #[day_macros::test(day_core)]
    fn prompt_native_dismiss() -> Case {
        Case::new()
            .requires(Cap::Dialogs)
            .proves_cap(Cap::Dialogs)
            .page(prompt_asker)
            .drive(|d: Drive| async move {
                d.dialogs_scripted(false).await?;
                d.tap("ask").await?;
                d.assert_presented(Some("Your name")).await?;
                d.respond(None).await?;
                d.assert_text("answer", "None").await
            })
    }

    #[day_macros::test(day_core)]
    fn alert_sheet_preserves_button_indices() -> Case {
        Case::new()
            .requires(Cap::Dialogs)
            .proves_cap(Cap::Dialogs)
            .page(|| {
                let answer = Signal::new(String::from("pending"));
                column((
                    label(move || answer.get()).id(day_core::id!(answer)),
                    button("Ask").id(day_core::id!(ask)).action(move || {
                        let set = answer.setter();
                        day_core::task(async move {
                            let value = Alert::new("Choose an action")
                                .sheet()
                                .button("First", 10)
                                .cancel("Cancel")
                                .destructive("Delete", 30)
                                .button("Last", 40)
                                .present()
                                .await;
                            set.set(format!("{value:?}"));
                        });
                    }),
                ))
            })
            .drive(|d: Drive| async move {
                d.dialogs_scripted(false).await?;
                d.tap("ask").await?;
                d.assert_presented(Some("Choose an action")).await?;
                d.respond(Some(2)).await?;
                d.assert_text("answer", "Some(30)").await?;
                d.tap("ask").await?;
                d.respond(Some(1)).await?;
                d.assert_text("answer", "None").await
            })
    }
}
