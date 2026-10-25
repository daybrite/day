// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
use block2::RcBlock;
use day_spec::{Point, ffi_guard, sidetable::SideTable, transfer::*};
use objc2::{
    DefinedClass, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{NSObjectProtocol, ProtocolObject},
};
use objc2_foundation::{
    NSArray, NSData, NSError, NSItemProvider, NSItemProviderRepresentationVisibility, NSObject,
    NSProgress, NSString,
};
use objc2_ui_kit::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::c_void,
    ptr::NonNull,
};
#[link(name = "MobileCoreServices", kind = "framework")]
unsafe extern "C" {
    static kUTTagClassMIMEType: *const c_void;
    fn UTTypeCreatePreferredIdentifierForTag(
        class: *const c_void,
        tag: *const c_void,
        conforms: *const c_void,
    ) -> *mut NSString;
    fn UTTypeCopyPreferredTagWithClass(uti: *const c_void, class: *const c_void) -> *mut NSString;
}
fn native(mime: &str) -> Retained<NSString> {
    let s = NSString::from_str(mime);
    unsafe {
        Retained::from_raw(UTTypeCreatePreferredIdentifierForTag(
            kUTTagClassMIMEType,
            (&*s as *const NSString).cast(),
            std::ptr::null(),
        ))
    }
    .unwrap_or(s)
}
fn mime(uti: &NSString) -> Option<String> {
    unsafe {
        Retained::from_raw(UTTypeCopyPreferredTagWithClass(
            (uti as *const NSString).cast(),
            kUTTagClassMIMEType,
        ))
    }
    .map(|s| s.to_string())
}
struct Ivars {
    source: RefCell<Option<Source>>,
    target: RefCell<Option<Target>>,
}
struct Pending {
    target: Target,
    at: Location,
    items: Vec<Option<Item>>,
    progress: Vec<Retained<NSProgress>>,
    total: usize,
}
day_core::tls_group! {
    static DELEGATES: SideTable<Retained<Delegate>> = SideTable::new();
    static PENDING: RefCell<HashMap<u64,Pending>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u64> = const { Cell::new(1) };
}
fn location(
    interaction: &UIDropInteraction,
    session: &ProtocolObject<dyn UIDropSession>,
) -> Option<Location> {
    let view = interaction.view()?;
    let p = unsafe { session.locationInView(&view) };
    let mut types = Vec::new();
    for item in unsafe { session.items() }.iter() {
        for ty in item.itemProvider().registeredTypeIdentifiers().iter() {
            if let Some(m) = mime(&ty)
                && !types.contains(&m)
            {
                types.push(m);
            }
        }
    }
    Some(Location {
        position: Point::new(p.x, p.y),
        types,
        allowed: vec![Operation::Copy],
        local: unsafe { session.localDragSession() }.is_some(),
    })
}
fn finish(token: u64, index: usize, mime: String, bytes: Option<Vec<u8>>) {
    let completed = PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        let p = pending.get_mut(&token)?;
        let Some(bytes) = bytes else {
            return pending.remove(&token).map(|p| (p, false));
        };
        p.total = p.total.saturating_add(bytes.len());
        if p.total > MAX_BYTES {
            return pending.remove(&token).map(|p| (p, false));
        }
        p.items[index] = Some(Item::new(vec![Representation::new(mime, bytes)]));
        if p.items.iter().all(Option::is_some) {
            pending.remove(&token).map(|p| (p, true))
        } else {
            None
        }
    });
    if let Some((p, ok)) = completed {
        for progress in &p.progress {
            progress.cancel();
        }
        if ok {
            let items: Vec<_> = p.items.into_iter().flatten().collect();
            let offer = if items.len() == 1 && items[0].get(BUNDLE_MIME).is_some() {
                Offer::decode(items[0].get(BUNDLE_MIME).unwrap())
            } else {
                Some(Offer { items })
            };
            if let Some(offer) = offer {
                p.target.deliver(p.at, offer);
            }
        }
    }
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[name = "DayTransferUIKitDelegate"]
    #[ivars=Ivars]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl UIDragInteractionDelegate for Delegate {
        #[unsafe(method_id(dragInteraction:itemsForBeginningSession:))]
        fn begin(
            &self,
            interaction: &UIDragInteraction,
            session: &ProtocolObject<dyn UIDragSession>,
        ) -> Retained<NSArray<UIDragItem>> {
            ffi_guard::contain(NSArray::new(), || {
                let Some(view) = interaction.view() else {
                    return NSArray::new();
                };
                let p = unsafe { session.locationInView(&view) };
                let source = self.ivars().source.borrow().clone();
                let Some(offer) = source.and_then(|f| f(Point::new(p.x, p.y))) else {
                    return NSArray::new();
                };
                let Some(packet) = offer.encode() else {
                    return NSArray::new();
                };
                let provider = NSItemProvider::new();
                let mut reps = vec![Representation::new(BUNDLE_MIME, packet)];
                if let Some(item) = offer.items.first() {
                    reps.extend(item.representations.clone());
                }
                for rep in reps {
                    let bytes = rep.bytes;
                    let block = RcBlock::new(
                        move |completion: NonNull<
                            block2::DynBlock<dyn Fn(*mut NSData, *mut NSError)>,
                        >|
                              -> *mut NSProgress {
                            let data = NSData::with_bytes(&bytes);
                            unsafe {
                                completion.as_ref().call((
                                    Retained::as_ptr(&data).cast_mut(),
                                    std::ptr::null_mut(),
                                ));
                            }
                            std::ptr::null_mut()
                        },
                    );
                    unsafe {
                        provider
                            .registerDataRepresentationForTypeIdentifier_visibility_loadHandler(
                                &native(&rep.mime),
                                NSItemProviderRepresentationVisibility::All,
                                &block,
                            );
                    }
                }
                let item = unsafe {
                    UIDragItem::initWithItemProvider(UIDragItem::alloc(self.mtm()), &provider)
                };
                NSArray::from_retained_slice(&[item])
            })
        }
    }
    unsafe impl UIDropInteractionDelegate for Delegate {
        #[unsafe(method_id(dropInteraction:sessionDidUpdate:))]
        fn update(
            &self,
            interaction: &UIDropInteraction,
            session: &ProtocolObject<dyn UIDropSession>,
        ) -> Retained<UIDropProposal> {
            let target = self.ivars().target.borrow().clone();
            let accepted = target
                .zip(location(interaction, session))
                .is_some_and(|(t, at)| t.proposal(&at) == Operation::Copy);
            UIDropProposal::initWithDropOperation(
                UIDropProposal::alloc(self.mtm()),
                if accepted {
                    UIDropOperation::Copy
                } else {
                    UIDropOperation::Forbidden
                },
            )
        }
        #[unsafe(method(dropInteraction:performDrop:))]
        fn receive(
            &self,
            interaction: &UIDropInteraction,
            session: &ProtocolObject<dyn UIDropSession>,
        ) {
            ffi_guard::contain((), || {
                let Some(target) = self.ivars().target.borrow().clone() else {
                    return;
                };
                let Some(at) = location(interaction, session) else {
                    return;
                };
                if target.proposal(&at) != Operation::Copy {
                    return;
                }
                let items = unsafe { session.items() };
                if items.is_empty() || items.len() > MAX_ITEMS {
                    return;
                }
                let mut reads = Vec::new();
                for item in items.iter() {
                    let provider = item.itemProvider();
                    let types = provider.registeredTypeIdentifiers();
                    let found = std::iter::once(BUNDLE_MIME)
                        .chain(target.types.iter().map(String::as_str))
                        .find(|m| types.containsObject(&native(m)));
                    let Some(m) = found else {
                        return;
                    };
                    reads.push((provider, m.to_owned()));
                    if m == BUNDLE_MIME {
                        break;
                    }
                }
                let token = NEXT.with(|n| {
                    let t = n.get();
                    n.set(t + 1);
                    t
                });
                PENDING.with(|p| {
                    p.borrow_mut().insert(
                        token,
                        Pending {
                            target,
                            at,
                            items: vec![None; reads.len()],
                            progress: Vec::new(),
                            total: 0,
                        },
                    )
                });
                for (index, (provider, mime)) in reads.into_iter().enumerate() {
                    let ty = native(&mime);
                    let block = RcBlock::new(move |data: *mut NSData, _error: *mut NSError| {
                        let bytes = unsafe { data.as_ref() }
                            .filter(|d| d.len() <= MAX_BYTES)
                            .map(NSData::to_vec);
                        let mime = mime.clone();
                        dispatch2::DispatchQueue::main()
                            .exec_async(move || finish(token, index, mime, bytes));
                    });
                    let progress = unsafe {
                        provider
                            .loadDataRepresentationForTypeIdentifier_completionHandler(&ty, &block)
                    };
                    PENDING.with(|p| {
                        if let Some(p) = p.borrow_mut().get_mut(&token) {
                            p.progress.push(progress);
                        }
                    });
                }
                let when = dispatch2::DispatchTime::try_from(std::time::Duration::from_secs(30))
                    .unwrap_or(dispatch2::DispatchTime::NOW);
                let _ = dispatch2::DispatchQueue::main().after(when, move || {
                    if let Some(p) = PENDING.with(|p| p.borrow_mut().remove(&token)) {
                        for progress in p.progress {
                            progress.cancel();
                        }
                    }
                });
            });
        }
    }
);
fn delegate(view: &UIView) -> Retained<Delegate> {
    let key = view as *const UIView as usize;
    if let Some(d) = DELEGATES.with(|t| t.get(key)) {
        return d;
    }
    let d = Delegate::alloc(view.mtm()).set_ivars(Ivars {
        source: RefCell::new(None),
        target: RefCell::new(None),
    });
    let d: Retained<Delegate> = unsafe { msg_send![super(d), init] };
    DELEGATES.with(|t| t.insert(key, d.clone()));
    d
}
pub fn source(view: &UIView, source: Source) {
    let d = delegate(view);
    *d.ivars().source.borrow_mut() = Some(source);
    let interaction = UIDragInteraction::initWithDelegate(
        UIDragInteraction::alloc(view.mtm()),
        ProtocolObject::from_ref(&*d),
    );
    interaction.setEnabled(true);
    view.addInteraction(ProtocolObject::from_ref(&*interaction));
}
pub fn target(view: &UIView, target: Target) {
    let d = delegate(view);
    *d.ivars().target.borrow_mut() = Some(target);
    let interaction = UIDropInteraction::initWithDelegate(
        UIDropInteraction::alloc(view.mtm()),
        ProtocolObject::from_ref(&*d),
    );
    view.addInteraction(ProtocolObject::from_ref(&*interaction));
}
