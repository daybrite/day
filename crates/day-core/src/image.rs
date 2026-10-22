// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Raster images (docs/images.md): decoding encoded bytes into a native image, reading what it
//! is, encoding it back, and releasing it again. Rendering a canvas drawing into a bitmap or a
//! PDF (docs/canvas.md "Exporting") lives here too: a rendered bitmap is the same [`Bitmap`] a
//! decode returns, held in the same toolkit table.
//!
//! The shape mirrors presentation (`present.rs`): a request id, a pending registry, and a
//! completion that arrives as an [`Event`] the backend raises. It is request-shaped rather than
//! blocking because one backend can never answer synchronously (a browser decodes through
//! `createImageBitmap`, which is a promise), and one async-shaped API everywhere beats a
//! synchronous one that is a lie on the web (docs/async.md rule 3: a callback and a future,
//! never a runtime).
//!
//! Everything here is thread-local and `!Send`, like the rest of day-core: Day has one UI thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use day_spec::{
    BitmapId, BitmapInfo, Cap, DrawOp, EncodeSpec, ExportError, ImageError, ImageFormat,
    ImageProperties, PdfInfo, PdfPage, Size,
};

use crate::with_tree;

day_reactive::tls_slots! {
    image;
    /// Keyed by request id, carrying the bitmap id day-core minted for it: the completion brings
    /// only what the decoder read, never an id, so nothing a backend answers can name a handle
    /// to something else.
    static PENDING_DECODE: RefCell<HashMap<u64, (BitmapId, Pending<(BitmapId, BitmapInfo)>)>> =
        RefCell::new(HashMap::new());
    static PENDING_ENCODE: RefCell<HashMap<u64, Pending<Vec<u8>>>> = RefCell::new(HashMap::new());
    /// Canvas exports, keyed like the decodes: a rendered bitmap's id is minted here too.
    static PENDING_BITMAP: RefCell<HashMap<u64, (BitmapId, Waiting<(BitmapId, BitmapInfo), ExportError>)>> =
        RefCell::new(HashMap::new());
    static PENDING_PDF: RefCell<HashMap<u64, Waiting<Vec<u8>, ExportError>>> =
        RefCell::new(HashMap::new());
    /// Releases that arrived while the tree was borrowed (a handler dropping its last handle
    /// inside `collect_and_release`), drained at the top of the next pump, once the borrow ends.
    static DEFERRED_RELEASE: RefCell<Vec<BitmapId>> = RefCell::new(Vec::new());
    static NEXT_REQ: Cell<u64> = const { Cell::new(1) };
    /// [`canvas_exports`]'s answer for the installed tree, read once: the caps and the encode
    /// list never change under a running toolkit, and a context menu built inside the tree's
    /// own borrow must still be able to ask.
    static EXPORTS: Cell<Option<CanvasExports>> = const { Cell::new(None) };
    /// Ids are minted here, not by the backends: day-core hands the id to the backend with the
    /// bytes, so the backend's side table is keyed by something the app can name afterwards.
    /// Never reused, so a completion arriving after a release finds nothing and does nothing.
    static NEXT_BITMAP: Cell<u64> = const { Cell::new(1) };
}

/// A caller waiting by CALLBACK rather than by future (docs/async.md rule 3): what
/// `decode_async` / `encode_async` hand over in place of an `.await`.
///
/// Named rather than spelled out at each use so the three signatures below read as one idea,
/// and not `Done<T>`, which is day-bridge's callback tier and a different thing.
type ImageDone<T, E = ImageError> = Box<dyn FnOnce(Result<T, E>)>;

/// One waiting caller: either a future parked on a `Waker`, or a plain callback. The error is
/// [`ImageError`] for the image duties and [`ExportError`] for canvas exports.
struct Waiting<T, E> {
    shared: Rc<Shared<T, E>>,
    callback: Option<ImageDone<T, E>>,
}

/// The image duties' waiter, named so their signatures read as before.
type Pending<T> = Waiting<T, ImageError>;

struct Shared<T, E = ImageError> {
    result: RefCell<Option<Result<T, E>>>,
    waker: RefCell<Option<Waker>>,
    /// The future was dropped before its answer came. A decode that completes afterwards has
    /// nobody to hand its image to, so the completion releases it instead of parking it.
    cancelled: Cell<bool>,
}

impl<T, E> Default for Shared<T, E> {
    fn default() -> Self {
        Shared {
            result: RefCell::new(None),
            waker: RefCell::new(None),
            cancelled: Cell::new(false),
        }
    }
}

fn next_req() -> u64 {
    NEXT_REQ.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    })
}

fn next_bitmap() -> BitmapId {
    BitmapId(NEXT_BITMAP.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    }))
}

// ---------------------------------------------------------------------------
// The handle
// ---------------------------------------------------------------------------

/// A decoded image the toolkit holds, released when the last handle drops.
///
/// Cheap to clone (an `Rc`), so several nodes and the canvas can draw the same decode. `!Send`
/// like everything else that talks to the toolkit.
#[derive(Clone)]
pub struct Bitmap(Rc<BitmapInner>);

struct BitmapInner {
    id: BitmapId,
    info: BitmapInfo,
}

impl Drop for BitmapInner {
    fn drop(&mut self) {
        release(self.id);
    }
}

/// Release a toolkit image now if the tree is free, and at the next pump if it is not.
///
/// A handle can die inside the tree's own borrow: `collect_and_release` drops a removed node's
/// handlers, and a handler may own the last clone. A plain `with_tree` there would panic on the
/// re-borrow, so the id is queued and [`flush_deferred_releases`] runs it once the borrow has
/// ended. The tree being absent (teardown, or a test that never mounted one) is the other way
/// to have nothing to release, and that is the no-op the duty promises.
fn release(id: BitmapId) {
    if crate::tree_absent() {
        return;
    }
    if crate::with_tree_if_free(|t| t.release_image(id)).is_none() {
        DEFERRED_RELEASE.with(|q| q.borrow_mut().push(id));
        crate::request_pump();
    }
}

/// Run the releases that could not run inline. Called at the top of every pump, which begins
/// only once the tree borrow has ended; a pump nested inside a still-borrowed tree keeps them
/// queued rather than dropping them.
pub(crate) fn flush_deferred_releases() {
    let ids: Vec<BitmapId> = DEFERRED_RELEASE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    for id in ids {
        if crate::with_tree_if_free(|t| t.release_image(id)).is_none() {
            DEFERRED_RELEASE.with(|q| q.borrow_mut().push(id));
        }
    }
}

impl Bitmap {
    /// The id the toolkit knows this image by, which [`day_spec::ImageSource::Decoded`] carries.
    pub fn id(&self) -> BitmapId {
        self.0.id
    }

    /// Pixel size, scale, format and alpha, read once at decode (docs/images.md).
    pub fn info(&self) -> BitmapInfo {
        self.0.info
    }

    /// The metadata beyond the pixels: EXIF orientation, DPI, capture time. `None` where this
    /// backend ships no metadata reader; probe [`Cap::ImageProperties`] before offering an
    /// affordance that needs it.
    pub fn properties(&self) -> Option<ImageProperties> {
        let id = self.0.id;
        with_tree(|t| t.image_properties(id))
    }

    /// Encode back to bytes, in `spec`'s format. Await it inside `day::task`.
    pub fn encode(&self, spec: EncodeSpec) -> EncodeFuture {
        EncodeFuture {
            source: self.clone(),
            spec,
            shared: Rc::new(Shared::default()),
            started: false,
        }
    }

    /// [`Bitmap::encode`]'s callback half, for a call site that is not already async.
    pub fn encode_async(
        &self,
        spec: EncodeSpec,
        on_done: impl FnOnce(Result<Vec<u8>, ImageError>) + 'static,
    ) {
        if support(Cap::ImageEncode) == day_spec::Support::Unsupported {
            on_done(Err(ImageError::Unsupported));
            return;
        }
        // The clone rides in the callback so the toolkit's image outlives the encode however the
        // app juggles its own handle; a browser encodes on a later turn.
        let keep = self.clone();
        start_encode(
            self.0.id,
            spec,
            None,
            Some(Box::new(move |r| {
                drop(keep);
                on_done(r)
            })),
        );
    }
}

impl std::fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bitmap")
            .field("id", &self.0.id.0)
            .field("info", &self.0.info)
            .finish()
    }
}

/// Two handles are the same image when they name the same decode.
impl PartialEq for Bitmap {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Decode encoded image bytes (PNG/JPEG/…) with the platform's own decoder.
///
/// ```ignore
/// day::task(async move {
///     match day::decode_image(bytes).await {
///         Ok(bitmap) => shown.set(Some(bitmap)),
///         Err(e) => log::warn!("not an image: {e}"),
///     }
/// });
/// ```
///
/// `Err(ImageError::Unsupported)` where the backend has no byte decoder at all
/// ([`Cap::ImageDecode`]); `Err(ImageError::Decode)` where it has one and the bytes are not an
/// image it reads.
pub fn decode(bytes: Arc<Vec<u8>>) -> DecodeFuture {
    DecodeFuture {
        bytes: Some(bytes),
        shared: Rc::new(Shared::default()),
        started: false,
    }
}

/// [`decode`]'s callback half (docs/async.md rule 3), for a call site that is not already async.
pub fn decode_async(
    bytes: Arc<Vec<u8>>,
    on_done: impl FnOnce(Result<Bitmap, ImageError>) + 'static,
) {
    if support(Cap::ImageDecode) == day_spec::Support::Unsupported {
        on_done(Err(ImageError::Unsupported));
        return;
    }
    start_decode(
        &bytes,
        None,
        Some(Box::new(move |r| on_done(r.map(bitmap_from)))),
    );
}

/// Whether this toolkit can decode image bytes at all ([`Cap::ImageDecode`]).
pub fn image_decode_support() -> day_spec::Support {
    support(Cap::ImageDecode)
}

/// Whether this toolkit can encode a decoded image back to bytes ([`Cap::ImageEncode`]).
pub fn image_encode_support() -> day_spec::Support {
    support(Cap::ImageEncode)
}

/// Which formats [`Bitmap::encode`] can write here: the [`Cap::ImageEncode`]
/// counterpart of `font_families()`. Empty where encoding is unsupported.
pub fn image_encode_formats() -> Vec<ImageFormat> {
    with_tree(|t| t.image_encode_formats())
}

fn support(cap: Cap) -> day_spec::Support {
    with_tree(|t| t.capability(cap))
}

fn bitmap_from((id, info): (BitmapId, BitmapInfo)) -> Bitmap {
    Bitmap(Rc::new(BitmapInner { id, info }))
}

/// Shared by the future and the callback form: mint the id, park the waiter, ask the toolkit.
fn start_decode(
    bytes: &[u8],
    shared: Option<Rc<Shared<(BitmapId, BitmapInfo)>>>,
    callback: Option<ImageDone<(BitmapId, BitmapInfo)>>,
) -> u64 {
    let req = next_req();
    let id = next_bitmap();
    PENDING_DECODE.with(|p| {
        p.borrow_mut().insert(
            req,
            (
                id,
                Pending {
                    shared: shared.unwrap_or_default(),
                    callback,
                },
            ),
        )
    });
    with_tree(|t| t.decode_image(req, id, bytes));
    req
}

fn start_encode(
    id: BitmapId,
    spec: EncodeSpec,
    shared: Option<Rc<Shared<Vec<u8>>>>,
    callback: Option<ImageDone<Vec<u8>>>,
) -> u64 {
    let req = next_req();
    PENDING_ENCODE.with(|p| {
        p.borrow_mut().insert(
            req,
            Pending {
                shared: shared.unwrap_or_default(),
                callback,
            },
        )
    });
    with_tree(|t| t.encode_image(req, id, &spec));
    req
}

/// A decode in flight. Resolves with the decoded [`Bitmap`], or why it could not be.
pub struct DecodeFuture {
    bytes: Option<Arc<Vec<u8>>>,
    shared: Rc<Shared<(BitmapId, BitmapInfo)>>,
    started: bool,
}

impl Future for DecodeFuture {
    type Output = Result<Bitmap, ImageError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(result) = self.shared.result.borrow_mut().take() {
            return Poll::Ready(result.map(bitmap_from));
        }
        if !self.started {
            self.started = true;
            if support(Cap::ImageDecode) == day_spec::Support::Unsupported {
                return Poll::Ready(Err(ImageError::Unsupported));
            }
            let bytes = self.bytes.take().expect("decode bytes");
            let shared = self.shared.clone();
            start_decode(&bytes, Some(shared), None);
            // A backend that decodes inline has already answered by now; every backend but the
            // browser does, since `with_tree` pumps the completion on its way out. The waker is
            // not registered yet at that point, so `deliver`'s wake reaches nobody; take the
            // result here instead of parking forever. (Presentation never needed this: a native
            // modal cannot answer before it is shown.)
            if let Some(result) = self.shared.result.borrow_mut().take() {
                return Poll::Ready(result.map(bitmap_from));
            }
        }
        *self.shared.waker.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for DecodeFuture {
    fn drop(&mut self) {
        // Answered but never taken: no handle was built, so the toolkit's image would outlive
        // every owner. Release it here.
        if let Some(Ok((id, _))) = self.shared.result.borrow_mut().take() {
            release(id);
        }
        // Still in flight: the completion, when it comes, releases rather than parking a result
        // nobody will read (`resolve_image_decode`).
        self.shared.cancelled.set(true);
    }
}

/// An encode in flight. Resolves with the encoded bytes, or why they could not be produced.
pub struct EncodeFuture {
    /// Held for the encode's whole life: dropping the app's last handle mid-encode would
    /// otherwise release the very image the toolkit is still reading.
    source: Bitmap,
    spec: EncodeSpec,
    shared: Rc<Shared<Vec<u8>>>,
    started: bool,
}

impl Future for EncodeFuture {
    type Output = Result<Vec<u8>, ImageError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(result) = self.shared.result.borrow_mut().take() {
            return Poll::Ready(result);
        }
        if !self.started {
            self.started = true;
            if support(Cap::ImageEncode) == day_spec::Support::Unsupported {
                return Poll::Ready(Err(ImageError::Unsupported));
            }
            let shared = self.shared.clone();
            start_encode(self.source.id(), self.spec, Some(shared), None);
            // Answered inline; see `DecodeFuture::poll`.
            if let Some(result) = self.shared.result.borrow_mut().take() {
                return Poll::Ready(result);
            }
        }
        *self.shared.waker.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

// ---------------------------------------------------------------------------
// Completions (pumped from `Event::ImageDecoded` / `Event::ImageEncoded`)
// ---------------------------------------------------------------------------

/// Deliver a decode answer. Called from `pump_events`; a request already answered, cancelled, or
/// never issued finds nothing and does nothing, the same contract presentation has.
pub fn resolve_image_decode(req: u64, result: Result<BitmapInfo, ImageError>) {
    let Some((id, entry)) = PENDING_DECODE.with(|p| p.borrow_mut().remove(&req)) else {
        return;
    };
    // The future is gone (a browser decode outliving the task that asked): a success has nobody
    // to own it, so it is released here rather than parked as a result no one will read.
    if entry.callback.is_none() && entry.shared.cancelled.get() {
        if result.is_ok() {
            release(id);
        }
        return;
    }
    deliver(entry, result.map(|info| (id, info)));
}

/// Deliver an encode answer (see [`resolve_image_decode`]).
pub fn resolve_image_encode(req: u64, result: Result<Vec<u8>, ImageError>) {
    let Some(entry) = PENDING_ENCODE.with(|p| p.borrow_mut().remove(&req)) else {
        return;
    };
    deliver(entry, result);
}

fn deliver<T, E>(entry: Waiting<T, E>, value: Result<T, E>) {
    if let Some(cb) = entry.callback {
        cb(value);
        return;
    }
    *entry.shared.result.borrow_mut() = Some(value);
    if let Some(waker) = entry.shared.waker.borrow_mut().take() {
        waker.wake();
    }
}

/// Resolve every pending request (tests, and `uninstall_tree`): the tree that would have answered
/// is going away, so each waiter gets `Unsupported` (there is no backend left to ask) rather
/// than staying parked forever. Queued releases go too: their toolkit is the one being torn down.
pub(crate) fn reset() {
    let decodes: Vec<_> =
        PENDING_DECODE.with(|p| p.borrow_mut().drain().map(|(_, (_, e))| e).collect());
    for entry in decodes {
        deliver(entry, Err(ImageError::Unsupported));
    }
    let encodes: Vec<_> = PENDING_ENCODE.with(|p| p.borrow_mut().drain().map(|(_, e)| e).collect());
    for entry in encodes {
        deliver(entry, Err(ImageError::Unsupported));
    }
    let bitmaps: Vec<_> =
        PENDING_BITMAP.with(|p| p.borrow_mut().drain().map(|(_, (_, e))| e).collect());
    for entry in bitmaps {
        deliver(entry, Err(ExportError::Unsupported));
    }
    let pdfs: Vec<_> = PENDING_PDF.with(|p| p.borrow_mut().drain().map(|(_, e)| e).collect());
    for entry in pdfs {
        deliver(entry, Err(ExportError::Unsupported));
    }
    DEFERRED_RELEASE.with(|q| q.borrow_mut().clear());
    EXPORTS.with(|c| c.set(None));
}

// ---------------------------------------------------------------------------
// Canvas export (docs/canvas.md "Exporting")
// ---------------------------------------------------------------------------

/// What this toolkit can export a canvas drawing to ([`Cap::CanvasBitmap`],
/// [`Cap::CanvasPdf`]). Read it to enable or hide export commands, the way
/// [`image_encode_formats`] decides which formats to offer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CanvasExports {
    /// [`render_bitmap`] works here, and with it PNG export wherever [`Bitmap::encode`] writes
    /// PNG (see [`CanvasExports::png`]).
    pub bitmap: bool,
    /// [`render_pdf`] works here: the platform has a native PDF writer.
    pub pdf: bool,
    png: bool,
}

impl CanvasExports {
    /// Whether a drawing can become PNG bytes here: a rendered bitmap plus a PNG encoder.
    pub fn png(&self) -> bool {
        self.png
    }
}

/// What this toolkit can export a canvas drawing to.
///
/// Safe to call anywhere, including from a context menu that is built while the tree is busy
/// summoning it: the answer is read once per installed tree and kept. Before a tree is
/// installed, and in the rare case its first read lands inside a borrow, everything answers
/// `false`.
pub fn canvas_exports() -> CanvasExports {
    if let Some(known) = EXPORTS.with(|c| c.get()) {
        return known;
    }
    let Some(read) = crate::with_tree_if_free(|t| {
        let yes = |cap| t.capability(cap) != day_spec::Support::Unsupported;
        let bitmap = yes(Cap::CanvasBitmap);
        CanvasExports {
            bitmap,
            pdf: yes(Cap::CanvasPdf),
            png: bitmap && t.image_encode_formats().contains(&ImageFormat::Png),
        }
    }) else {
        return CanvasExports::default();
    };
    EXPORTS.with(|c| c.set(Some(read)));
    read
}

/// `size` points at `scale` is something a toolkit can be asked to render: positive and
/// finite, and within [`day_spec::EXPORT_MAX_SIDE`] / [`day_spec::EXPORT_MAX_PIXELS`].
fn check_bitmap(size: Size, scale: f64) -> Result<(), ExportError> {
    let positive = |v: f64| v.is_finite() && v > 0.0;
    if !positive(size.width) || !positive(size.height) || !positive(scale) {
        return Err(ExportError::Invalid);
    }
    let (width, height) = day_spec::export_pixels(size, scale);
    if width > day_spec::EXPORT_MAX_SIDE
        || height > day_spec::EXPORT_MAX_SIDE
        || u64::from(width) * u64::from(height) > day_spec::EXPORT_MAX_PIXELS
    {
        return Err(ExportError::TooLarge { width, height });
    }
    Ok(())
}

fn check_pdf(pages: &[PdfPage]) -> Result<(), ExportError> {
    let positive = |v: f64| v.is_finite() && v > 0.0;
    if pages.is_empty()
        || pages
            .iter()
            .any(|p| !positive(p.size.width) || !positive(p.size.height))
    {
        return Err(ExportError::Invalid);
    }
    Ok(())
}

/// The check every bitmap export runs before asking the toolkit: the cap, then the size.
fn admit_bitmap(size: Size, scale: f64) -> Result<(), ExportError> {
    if support(Cap::CanvasBitmap) == day_spec::Support::Unsupported {
        return Err(ExportError::Unsupported);
    }
    check_bitmap(size, scale)
}

fn admit_pdf(pages: &[PdfPage]) -> Result<(), ExportError> {
    if support(Cap::CanvasPdf) == day_spec::Support::Unsupported {
        return Err(ExportError::Unsupported);
    }
    check_pdf(pages)
}

/// Replay `ops` into an offscreen bitmap of `size` points at `scale` pixels per point, through
/// the toolkit's own canvas painter. Resolves with a [`Bitmap`] like a decode does, so
/// `image(&bitmap)`, `Draw::image` and [`Bitmap::encode`] all take it. Apps reach this through
/// `Drawing::to_bitmap`.
pub fn render_bitmap(ops: Vec<DrawOp>, size: Size, scale: f64) -> RenderBitmap {
    RenderBitmap {
        job: Some((ops, size, scale)),
        shared: Rc::new(Shared::default()),
        started: false,
    }
}

/// [`render_bitmap`]'s callback half (docs/async.md rule 3).
pub fn render_bitmap_async(
    ops: Vec<DrawOp>,
    size: Size,
    scale: f64,
    on_done: impl FnOnce(Result<Bitmap, ExportError>) + 'static,
) {
    if let Err(e) = admit_bitmap(size, scale) {
        on_done(Err(e));
        return;
    }
    start_bitmap(
        &ops,
        size,
        scale,
        None,
        Some(Box::new(move |r| on_done(r.map(bitmap_from)))),
    );
}

/// Replay each page into one PDF document through the platform's own PDF writer. Resolves with
/// the document's bytes. Apps reach this through `Drawing::to_pdf` and `PdfDocument::render`.
pub fn render_pdf(pages: Vec<PdfPage>, info: PdfInfo) -> RenderPdf {
    RenderPdf {
        job: Some((pages, info)),
        shared: Rc::new(Shared::default()),
        started: false,
    }
}

/// [`render_pdf`]'s callback half (docs/async.md rule 3).
pub fn render_pdf_async(
    pages: Vec<PdfPage>,
    info: PdfInfo,
    on_done: impl FnOnce(Result<Vec<u8>, ExportError>) + 'static,
) {
    if let Err(e) = admit_pdf(&pages) {
        on_done(Err(e));
        return;
    }
    start_pdf(&pages, &info, None, Some(Box::new(on_done)));
}

fn start_bitmap(
    ops: &[DrawOp],
    size: Size,
    scale: f64,
    shared: Option<Rc<Shared<(BitmapId, BitmapInfo), ExportError>>>,
    callback: Option<ImageDone<(BitmapId, BitmapInfo), ExportError>>,
) {
    let req = next_req();
    let id = next_bitmap();
    PENDING_BITMAP.with(|p| {
        p.borrow_mut().insert(
            req,
            (
                id,
                Waiting {
                    shared: shared.unwrap_or_default(),
                    callback,
                },
            ),
        )
    });
    with_tree(|t| t.render_bitmap(req, id, ops, size, scale));
}

fn start_pdf(
    pages: &[PdfPage],
    info: &PdfInfo,
    shared: Option<Rc<Shared<Vec<u8>, ExportError>>>,
    callback: Option<ImageDone<Vec<u8>, ExportError>>,
) {
    let req = next_req();
    PENDING_PDF.with(|p| {
        p.borrow_mut().insert(
            req,
            Waiting {
                shared: shared.unwrap_or_default(),
                callback,
            },
        )
    });
    with_tree(|t| t.render_pdf(req, pages, info));
}

/// A bitmap render in flight. Resolves with the rendered [`Bitmap`], or why it could not be.
/// Nothing is asked of the toolkit until the first poll.
pub struct RenderBitmap {
    job: Option<(Vec<DrawOp>, Size, f64)>,
    shared: Rc<Shared<(BitmapId, BitmapInfo), ExportError>>,
    started: bool,
}

impl Future for RenderBitmap {
    type Output = Result<Bitmap, ExportError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(result) = self.shared.result.borrow_mut().take() {
            return Poll::Ready(result.map(bitmap_from));
        }
        if !self.started {
            self.started = true;
            let Some((ops, size, scale)) = self.job.take() else {
                return Poll::Ready(Err(ExportError::Invalid));
            };
            if let Err(e) = admit_bitmap(size, scale) {
                return Poll::Ready(Err(e));
            }
            start_bitmap(&ops, size, scale, Some(self.shared.clone()), None);
            // Answered inline everywhere but the browser; see `DecodeFuture::poll`.
            if let Some(result) = self.shared.result.borrow_mut().take() {
                return Poll::Ready(result.map(bitmap_from));
            }
        }
        *self.shared.waker.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for RenderBitmap {
    fn drop(&mut self) {
        // The same two cases as `DecodeFuture`: answered but never taken, or still in flight.
        if let Some(Ok((id, _))) = self.shared.result.borrow_mut().take() {
            release(id);
        }
        self.shared.cancelled.set(true);
    }
}

/// A PDF render in flight. Resolves with the document's bytes, or why they could not be
/// produced. Nothing is asked of the toolkit until the first poll.
pub struct RenderPdf {
    job: Option<(Vec<PdfPage>, PdfInfo)>,
    shared: Rc<Shared<Vec<u8>, ExportError>>,
    started: bool,
}

impl Future for RenderPdf {
    type Output = Result<Vec<u8>, ExportError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(result) = self.shared.result.borrow_mut().take() {
            return Poll::Ready(result);
        }
        if !self.started {
            self.started = true;
            let Some((pages, info)) = self.job.take() else {
                return Poll::Ready(Err(ExportError::Invalid));
            };
            if let Err(e) = admit_pdf(&pages) {
                return Poll::Ready(Err(e));
            }
            start_pdf(&pages, &info, Some(self.shared.clone()), None);
            if let Some(result) = self.shared.result.borrow_mut().take() {
                return Poll::Ready(result);
            }
        }
        *self.shared.waker.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Deliver a bitmap render's answer (`Event::BitmapRendered`). The same contract as
/// [`resolve_image_decode`], including releasing a success nobody is waiting for.
pub fn resolve_bitmap_render(req: u64, result: Result<BitmapInfo, ExportError>) {
    let Some((id, entry)) = PENDING_BITMAP.with(|p| p.borrow_mut().remove(&req)) else {
        return;
    };
    if entry.callback.is_none() && entry.shared.cancelled.get() {
        if result.is_ok() {
            release(id);
        }
        return;
    }
    deliver(entry, result.map(|info| (id, info)));
}

/// Deliver a PDF render's answer (`Event::PdfRendered`).
pub fn resolve_pdf_render(req: u64, result: Result<Vec<u8>, ExportError>) {
    let Some(entry) = PENDING_PDF.with(|p| p.borrow_mut().remove(&req)) else {
        return;
    };
    deliver(entry, result);
}

#[cfg(test)]
mod export_tests {
    use super::*;

    #[test]
    fn bitmap_sizes_are_checked_before_the_toolkit() {
        let size = Size::new(100.0, 50.0);
        assert_eq!(check_bitmap(size, 2.0), Ok(()));
        assert_eq!(
            check_bitmap(Size::new(0.0, 50.0), 1.0),
            Err(ExportError::Invalid)
        );
        assert_eq!(check_bitmap(size, f64::NAN), Err(ExportError::Invalid));
        assert_eq!(check_bitmap(size, -1.0), Err(ExportError::Invalid));
        assert_eq!(
            check_bitmap(Size::new(16_385.0, 1.0), 1.0),
            Err(ExportError::TooLarge {
                width: 16_385,
                height: 1
            })
        );
        // Each side fits, the total does not: 10,000 × 10,000 is 100 megapixels.
        assert_eq!(
            check_bitmap(Size::new(5_000.0, 5_000.0), 2.0),
            Err(ExportError::TooLarge {
                width: 10_000,
                height: 10_000
            })
        );
        // Partial pixels round up, so the limit counts the bitmap actually allocated.
        assert_eq!(day_spec::export_pixels(Size::new(10.2, 3.0), 2.0), (21, 6));
    }

    #[test]
    fn a_pdf_needs_pages_with_a_size() {
        assert_eq!(check_pdf(&[]), Err(ExportError::Invalid));
        let page = |w, h| PdfPage {
            size: Size::new(w, h),
            ops: Vec::new(),
        };
        assert_eq!(check_pdf(&[page(612.0, 792.0)]), Ok(()));
        assert_eq!(
            check_pdf(&[page(612.0, 792.0), page(0.0, 10.0)]),
            Err(ExportError::Invalid)
        );
    }
}

#[cfg(test)]
mod teardown_tests {
    use super::*;

    #[test]
    fn bitmap_drop_during_core_tls_teardown() {
        const CHILD: &str = "DAY_CORE_BITMAP_TEARDOWN_CHILD";
        if std::env::var_os(CHILD).is_some() {
            struct TeardownBitmap {
                bitmap: Option<Bitmap>,
                dropped: std::sync::mpsc::Sender<()>,
            }
            impl Drop for TeardownBitmap {
                fn drop(&mut self) {
                    // LocalKey access must fail while its own destructor runs. This does not
                    // depend on the platform's ordering of unrelated TLS destructors.
                    assert!(crate::try_tls_root(|_| ()).is_err());
                    drop(self.bitmap.take());
                    self.dropped.send(()).unwrap();
                }
            }
            let (dropped, observed) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let held = TeardownBitmap {
                    bitmap: Some(bitmap_from((BitmapId(1), BitmapInfo::default()))),
                    dropped,
                };
                // A pending callback owns the bitmap until the core TLS root is destroyed.
                // The callback is never invoked: dropping the registry drops its captures.
                PENDING_ENCODE.with(|pending| {
                    pending.borrow_mut().insert(
                        1,
                        Pending {
                            shared: Rc::new(Shared::default()),
                            callback: Some(Box::new(move |_| drop(held))),
                        },
                    );
                });
                assert!(crate::try_tls_root(|_| ()).is_ok());
            })
            .join()
            .unwrap();
            // Prove the destructor actually exercised the release path, rather than allowing
            // a platform that skipped TLS cleanup to pass silently.
            observed
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("bitmap was not dropped during core TLS teardown");
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "image::teardown_tests::bitmap_drop_during_core_tls_teardown",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
