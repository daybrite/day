// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Raster images (docs/images.md): decoding encoded bytes into a native image, reading what it
//! is, encoding it back, and releasing it again.
//!
//! The shape mirrors presentation (`present.rs`): a request id, a pending registry, and a
//! completion that arrives as an [`Event`] the backend raises. It is request-shaped rather than
//! blocking because one backend can never answer synchronously — a browser decodes through
//! `createImageBitmap`, which is a promise — and one async-shaped API everywhere beats a
//! synchronous one that is a lie on the web (docs/async.md rule 3: a callback AND a future,
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

use day_spec::{BitmapId, BitmapInfo, Cap, EncodeSpec, ImageError, ImageFormat, ImageProperties};

use crate::with_tree;

day_reactive::tls_slots! {
    image;
    /// Keyed by request id, carrying the bitmap id day-core minted for it: the completion brings
    /// only what the decoder read, never an id, so nothing a backend answers can name a handle
    /// to something else.
    static PENDING_DECODE: RefCell<HashMap<u64, (BitmapId, Pending<(BitmapId, BitmapInfo)>)>> =
        RefCell::new(HashMap::new());
    static PENDING_ENCODE: RefCell<HashMap<u64, Pending<Vec<u8>>>> = RefCell::new(HashMap::new());
    /// Releases that arrived while the tree was borrowed — a handler dropping its last handle
    /// inside `collect_and_release` — drained at the top of the next pump, once the borrow ends.
    static DEFERRED_RELEASE: RefCell<Vec<BitmapId>> = RefCell::new(Vec::new());
    static NEXT_REQ: Cell<u64> = const { Cell::new(1) };
    /// Ids are minted HERE, not by the backends: day-core hands the id to the backend with the
    /// bytes, so the backend's side table is keyed by something the app can name afterwards.
    /// Never reused, so a completion arriving after a release finds nothing and does nothing.
    static NEXT_BITMAP: Cell<u64> = const { Cell::new(1) };
}

/// A caller waiting by CALLBACK rather than by future (docs/async.md rule 3): what
/// `decode_async` / `encode_async` hand over in place of an `.await`.
///
/// Named rather than spelled out at each use so the three signatures below read as one idea —
/// and deliberately not `Done<T>`, which is day-bridge's own callback tier and a different thing.
type ImageDone<T> = Box<dyn FnOnce(Result<T, ImageError>)>;

/// One waiting caller: either a future parked on a `Waker`, or a plain callback.
struct Pending<T> {
    shared: Rc<Shared<T>>,
    callback: Option<ImageDone<T>>,
}

struct Shared<T> {
    result: RefCell<Option<Result<T, ImageError>>>,
    waker: RefCell<Option<Waker>>,
    /// The future was dropped before its answer came. A decode that completes afterwards has
    /// nobody to hand its image to, so the completion releases it instead of parking it.
    cancelled: Cell<bool>,
}

impl<T> Default for Shared<T> {
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
/// A handle can die INSIDE the tree's own borrow: `collect_and_release` drops a removed node's
/// handlers, and a handler may own the last clone. A plain `with_tree` there would panic on the
/// re-borrow, so the id is queued and [`flush_deferred_releases`] runs it once the borrow has
/// ended. The tree being absent — teardown, or a test that never mounted one — is the other way
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
    /// The id the toolkit knows this image by — what [`day_spec::ImageSource::Decoded`] carries.
    pub fn id(&self) -> BitmapId {
        self.0.id
    }

    /// Pixel size, scale, format and alpha, read once at decode (docs/images.md).
    pub fn info(&self) -> BitmapInfo {
        self.0.info
    }

    /// The metadata beyond the pixels — EXIF orientation, DPI, capture time. `None` where this
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
        // app juggles its own handle — a browser encodes on a later turn.
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

/// Which formats [`Bitmap::encode`] can actually write here — the [`Cap::ImageEncode`]
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
            // A backend that decodes INLINE has already answered by now — every backend but the
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
            // Answered inline — see `DecodeFuture::poll`.
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
/// never issued finds nothing and does nothing — the same contract presentation has.
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

fn deliver<T>(entry: Pending<T>, value: Result<T, ImageError>) {
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
/// is going away, so each waiter gets `Unsupported` — there is no backend left to ask — rather
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
    DEFERRED_RELEASE.with(|q| q.borrow_mut().clear());
}
