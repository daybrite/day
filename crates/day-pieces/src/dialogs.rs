//! Imperative, awaitable presentations: the `alert` / `confirm` / `prompt` dialogs and the
//! `open_file` / `save_file` system file pickers (`FileUrl`) — each returns a future you await
//! for the user's choice.

use std::hash::Hash;

use crate::*;

// ---------------------------------------------------------------------------
// Imperative presentation (docs/dialogs.md)
// ---------------------------------------------------------------------------

use std::future::{Future, IntoFuture};
use std::pin::Pin;

use day_spec::present::{ButtonRole, PresentButton, PresentResult, PresentSpec};

/// Boxed future the awaitable presenters resolve to — one alloc per dialog, negligible.
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
        confirm: day_l10n::t("day-ok"),
        cancel: day_l10n::t("day-cancel"),
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
        ok: day_l10n::t("day-ok"),
        cancel: day_l10n::t("day-cancel"),
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
/// [`FileUrl::read`] — backends copy a picked file into app storage first where the platform
/// requires it, so the local path "just works" everywhere.
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
    /// The locator as a filesystem path — `Some` for local paths (and `file://` URLs), `None`
    /// for opaque URIs such as Android's `content://`.
    pub fn local_path(&self) -> Option<std::path::PathBuf> {
        if self.0.contains("://") && !self.0.starts_with("file://") {
            return None;
        }
        let p = self.0.strip_prefix("file://").unwrap_or(&self.0);
        Some(std::path::PathBuf::from(p))
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
            Some(p) => std::fs::read(p),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "content:// URIs are not directly readable",
            )),
        }
    }
    /// Read the file as UTF-8 text.
    pub fn read_to_string(&self) -> std::io::Result<String> {
        match self.local_path() {
            Some(p) => std::fs::read_to_string(p),
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
        title: day_l10n::t("day-open"),
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
        title: day_l10n::t("day-save"),
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
            std::process::id(),
            sanitize_name(&self.suggested_name)
        ));
        if std::fs::write(&src, &self.data).is_err() {
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
                let _ = std::fs::remove_file(&src);
                return None;
            }
        };
        // Best-effort deliver the bytes to a local destination (desktop, and headless dayscript).
        // On Android the destination is a `content://` URI the backend already wrote (no
        // `local_path`, so the copy is skipped); iOS delivers via the document exporter.
        if let Some(p) = dest.local_path()
            && p != src
        {
            let _ = std::fs::copy(&src, &p);
        }
        let _ = std::fs::remove_file(&src);
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
