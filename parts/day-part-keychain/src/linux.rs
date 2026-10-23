// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Desktop Linux (GTK and Qt alike): the Secret Service, through libsecret.
//!
//! `org.freedesktop.secrets` on the session bus is the one store a desktop Linux app can rely on:
//! GNOME Keyring, KDE's ksecretd (KDE Frameworks 5.97 and later) and KeePassXC all serve it.
//! libsecret is its client library, and it is loaded at run time rather than linked, the way
//! day-part-sound loads libpulse: a linked library is a `DT_NEEDED` entry, and an app should start
//! on a machine without libsecret and report `Unsupported` there (docs/bridge.md "Linking").
//!
//! # Items
//!
//! One keyring item per key, in the default collection, under a schema of this crate's own
//! (`dev.daybrite.day.keychain`) with two attributes: `application` (the app id) and `key`. The
//! schema name is matched on lookups, so another libsecret client's items are never read as ours,
//! and the `application` attribute keeps two Day apps apart. The secret is stored as base64 text
//! with the `text/plain` content type, which every implementation behind the interface keeps
//! intact (see `base64.rs`).
//!
//! # Protection
//!
//! The Secret Service has one level: readable while the keyring is unlocked. `WhenUnlocked` and
//! `AfterFirstUnlock` both map to it. `UserPresence` is `Unsupported`: there is no API an app may
//! use to verify the person (fprintd is claimed by the login stack and binds nothing; polkit
//! authenticates on the system's behalf), so a `set` at that level fails rather than store an
//! item with a promise it cannot keep.
//!
//! # Blocking
//!
//! libsecret's `*_sync` calls spin a private main context, so they do not re-enter the toolkit's
//! loop, but they block the calling thread, and a locked keyring makes the service show its unlock
//! dialog during the call. The synchronous API here therefore blocks for as long as that dialog is
//! up; `get_with_presence` runs on a thread of its own, which is what UI code should use when the
//! keyring may be locked. A dismissed dialog reads as [`KeychainError::Locked`].

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::OnceLock;

use crate::{Entry, KeychainError, OnDone, Protection, Support};

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

const RTLD_LAZY: c_int = 1;

/// `SecretSchemaAttributeType`.
const ATTRIBUTE_STRING: c_int = 0;
/// `SecretSchemaFlags::SECRET_SCHEMA_NONE`: the schema name is matched on lookups.
const SCHEMA_NONE: c_int = 0;
/// `SecretSearchFlags::SECRET_SEARCH_ALL`.
const SEARCH_ALL: c_int = 1 << 1;
/// `SecretError` codes in the `secret-error` domain.
const ERROR_IS_LOCKED: c_int = 2;
const ERROR_NO_SUCH_OBJECT: c_int = 3;

/// The schema name; stable forever, since the Secret Service matches it on every lookup.
const SCHEMA_NAME: &CStr = c"dev.daybrite.day.keychain";
const ATTR_APPLICATION: &CStr = c"application";
const ATTR_KEY: &CStr = c"key";

/// `SecretSchemaAttribute`.
#[repr(C)]
struct SchemaAttribute {
    name: *const c_char,
    kind: c_int,
}

/// `SecretSchema`: the name, flags, 32 attribute slots terminated by a null name, and the
/// reserved tail libsecret keeps for itself.
#[repr(C)]
struct Schema {
    name: *const c_char,
    flags: c_int,
    attributes: [SchemaAttribute; 32],
    reserved: c_int,
    reserved_pointers: [*mut c_void; 7],
}

/// `GError`.
#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *mut c_char,
}

/// `GList`.
#[repr(C)]
struct GList {
    data: *mut c_void,
    next: *mut GList,
    prev: *mut GList,
}

type SchemaPtr = *const Schema;
type HashTable = *mut c_void;
type ErrorPtr = *mut *mut GError;

type StorevSync = unsafe extern "C" fn(
    SchemaPtr,
    HashTable,
    *const c_char,
    *const c_char,
    *const c_char,
    *mut c_void,
    ErrorPtr,
) -> c_int;
type LookupvSync = unsafe extern "C" fn(SchemaPtr, HashTable, *mut c_void, ErrorPtr) -> *mut c_char;
type ClearvSync = unsafe extern "C" fn(SchemaPtr, HashTable, *mut c_void, ErrorPtr) -> c_int;
type SearchvSync =
    unsafe extern "C" fn(SchemaPtr, HashTable, c_int, *mut c_void, ErrorPtr) -> *mut GList;
type ItemGetAttributes = unsafe extern "C" fn(*mut c_void) -> HashTable;
type PasswordFree = unsafe extern "C" fn(*mut c_char);
type ErrorQuark = unsafe extern "C" fn() -> u32;
type ServiceGetSync = unsafe extern "C" fn(c_int, *mut c_void, ErrorPtr) -> *mut c_void;
type HashTableNewFull =
    unsafe extern "C" fn(*const c_void, *const c_void, *const c_void, *const c_void) -> HashTable;
type HashTableInsert = unsafe extern "C" fn(HashTable, *mut c_void, *mut c_void) -> c_int;
type HashTableLookup = unsafe extern "C" fn(HashTable, *const c_void) -> *mut c_void;
type HashTableUnref = unsafe extern "C" fn(HashTable);
type ListFreeFull = unsafe extern "C" fn(*mut GList, *const c_void);
type ErrorFree = unsafe extern "C" fn(*mut GError);
type ObjectUnref = unsafe extern "C" fn(*mut c_void);

/// Every entry point this arm uses, resolved once. libsecret pulls GLib and GObject in as its
/// own dependencies, so their symbols come from the same process once it is loaded; they are
/// looked up through their own libraries so the intent is plain.
struct Lib {
    storev_sync: StorevSync,
    lookupv_sync: LookupvSync,
    clearv_sync: ClearvSync,
    searchv_sync: SearchvSync,
    item_get_attributes: ItemGetAttributes,
    password_free: PasswordFree,
    error_quark: ErrorQuark,
    service_get_sync: ServiceGetSync,
    hash_table_new_full: HashTableNewFull,
    hash_table_insert: HashTableInsert,
    hash_table_lookup: HashTableLookup,
    hash_table_unref: HashTableUnref,
    list_free_full: ListFreeFull,
    error_free: ErrorFree,
    object_unref: ObjectUnref,
    str_hash: *const c_void,
    str_equal: *const c_void,
    dbus_error_quark: ErrorQuark,
}

// SAFETY: the struct holds only function pointers and plain symbol addresses, valid for the life
// of the process once the libraries are loaded (they are never unloaded).
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

static LIB: OnceLock<Option<Lib>> = OnceLock::new();

impl Lib {
    fn load() -> Option<Lib> {
        // SAFETY: dlopen/dlsym with NUL-terminated names; each symbol is cast to the signature its
        // header declares, and a missing one abandons the whole arm.
        unsafe {
            let secret = dlopen(c"libsecret-1.so.0".as_ptr(), RTLD_LAZY);
            let glib = dlopen(c"libglib-2.0.so.0".as_ptr(), RTLD_LAZY);
            let gobject = dlopen(c"libgobject-2.0.so.0".as_ptr(), RTLD_LAZY);
            let gio = dlopen(c"libgio-2.0.so.0".as_ptr(), RTLD_LAZY);
            if secret.is_null() || glib.is_null() || gobject.is_null() || gio.is_null() {
                return None;
            }
            let sym = |lib: *mut c_void, name: &CStr| {
                let p = dlsym(lib, name.as_ptr());
                (!p.is_null()).then_some(p)
            };
            macro_rules! f {
                ($lib:expr, $name:literal, $ty:ty) => {
                    std::mem::transmute::<*mut c_void, $ty>(sym($lib, $name)?)
                };
            }
            Some(Lib {
                storev_sync: f!(secret, c"secret_password_storev_sync", StorevSync),
                lookupv_sync: f!(secret, c"secret_password_lookupv_sync", LookupvSync),
                clearv_sync: f!(secret, c"secret_password_clearv_sync", ClearvSync),
                searchv_sync: f!(secret, c"secret_password_searchv_sync", SearchvSync),
                item_get_attributes: f!(secret, c"secret_item_get_attributes", ItemGetAttributes),
                password_free: f!(secret, c"secret_password_free", PasswordFree),
                error_quark: f!(secret, c"secret_error_get_quark", ErrorQuark),
                service_get_sync: f!(secret, c"secret_service_get_sync", ServiceGetSync),
                hash_table_new_full: f!(glib, c"g_hash_table_new_full", HashTableNewFull),
                hash_table_insert: f!(glib, c"g_hash_table_insert", HashTableInsert),
                hash_table_lookup: f!(glib, c"g_hash_table_lookup", HashTableLookup),
                hash_table_unref: f!(glib, c"g_hash_table_unref", HashTableUnref),
                list_free_full: f!(glib, c"g_list_free_full", ListFreeFull),
                error_free: f!(glib, c"g_error_free", ErrorFree),
                object_unref: f!(gobject, c"g_object_unref", ObjectUnref),
                str_hash: sym(glib, c"g_str_hash")?,
                str_equal: sym(glib, c"g_str_equal")?,
                dbus_error_quark: f!(gio, c"g_dbus_error_quark", ErrorQuark),
            })
        }
    }

    fn get() -> Option<&'static Lib> {
        LIB.get_or_init(Lib::load).as_ref()
    }
}

/// This crate's schema, built once. The attribute names are `'static` C strings, so the struct
/// holds pointers that never dangle.
fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let mut attributes: [SchemaAttribute; 32] = std::array::from_fn(|_| SchemaAttribute {
            name: std::ptr::null(),
            kind: ATTRIBUTE_STRING,
        });
        attributes[0] = SchemaAttribute {
            name: ATTR_APPLICATION.as_ptr(),
            kind: ATTRIBUTE_STRING,
        };
        attributes[1] = SchemaAttribute {
            name: ATTR_KEY.as_ptr(),
            kind: ATTRIBUTE_STRING,
        };
        Schema {
            name: SCHEMA_NAME.as_ptr(),
            flags: SCHEMA_NONE,
            attributes,
            reserved: 0,
            reserved_pointers: [std::ptr::null_mut(); 7],
        }
    })
}

// SAFETY: a `Schema` is plain data and `'static` pointers into this binary's constants.
unsafe impl Sync for Schema {}
unsafe impl Send for Schema {}

/// A `GHashTable` of attributes whose strings this struct owns for the table's lifetime.
struct Attributes {
    lib: &'static Lib,
    table: HashTable,
    _owned: Vec<CString>,
}

impl Attributes {
    /// `application` (always) and `key` (when given).
    fn new(lib: &'static Lib, key: Option<&str>) -> Result<Attributes, KeychainError> {
        let app = CString::new(crate::app_id()).map_err(|_| KeychainError::InvalidKey)?;
        let mut owned = vec![app];
        let key = key
            .map(|k| CString::new(k).map_err(|_| KeychainError::InvalidKey))
            .transpose()?;
        // SAFETY: `g_hash_table_new_full` with string hashing and no destroy functions: the table
        // borrows the strings, which `owned` keeps alive until the table is unreffed in `Drop`.
        let table = unsafe {
            (lib.hash_table_new_full)(
                lib.str_hash,
                lib.str_equal,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if table.is_null() {
            return Err(KeychainError::Platform(
                "g_hash_table_new_full failed".into(),
            ));
        }
        unsafe {
            (lib.hash_table_insert)(
                table,
                ATTR_APPLICATION.as_ptr().cast_mut().cast(),
                owned[0].as_ptr().cast_mut().cast(),
            );
        }
        if let Some(key) = key {
            unsafe {
                (lib.hash_table_insert)(
                    table,
                    ATTR_KEY.as_ptr().cast_mut().cast(),
                    key.as_ptr().cast_mut().cast(),
                );
            }
            owned.push(key);
        }
        Ok(Attributes {
            lib,
            table,
            _owned: owned,
        })
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: the table came from `g_hash_table_new_full` and is released exactly once.
        unsafe { (self.lib.hash_table_unref)(self.table) };
    }
}

/// Read and free a `GError`, in the crate's vocabulary.
fn take_error(lib: &Lib, error: *mut GError) -> KeychainError {
    if error.is_null() {
        return KeychainError::Platform("libsecret failed without an error".into());
    }
    // SAFETY: a non-null GError out-parameter is a live, caller-owned error until freed here.
    unsafe {
        let domain = (*error).domain;
        let code = (*error).code;
        let message = if (*error).message.is_null() {
            String::new()
        } else {
            CStr::from_ptr((*error).message)
                .to_string_lossy()
                .into_owned()
        };
        (lib.error_free)(error);
        if domain == (lib.error_quark)() {
            match code {
                ERROR_IS_LOCKED => KeychainError::Locked,
                ERROR_NO_SUCH_OBJECT => KeychainError::NotFound,
                _ => KeychainError::Platform(message),
            }
        } else if domain == (lib.dbus_error_quark)() {
            // No service on the bus, or no bus at all (a headless session, a container).
            KeychainError::Unavailable
        } else {
            KeychainError::Platform(message)
        }
    }
}

/// Whether a Secret Service answers on this session. Asked without caching: a keyring daemon
/// may start after the app did.
fn service_reachable(lib: &Lib) -> bool {
    let mut error: *mut GError = std::ptr::null_mut();
    // SAFETY: `secret_service_get_sync` with no flags returns a cached proxy (not ours to
    // unref beyond the reference it hands back) or null with an error, freed below.
    unsafe {
        let service = (lib.service_get_sync)(0, std::ptr::null_mut(), &mut error);
        if service.is_null() {
            if !error.is_null() {
                (lib.error_free)(error);
            }
            return false;
        }
        (lib.object_unref)(service);
    }
    true
}

pub fn support() -> Support {
    match Lib::get() {
        Some(lib) if service_reachable(lib) => Support::Native,
        _ => Support::Unsupported,
    }
}

pub fn protection_support(protection: Protection) -> Support {
    match protection {
        Protection::UserPresence => Support::Unsupported,
        _ => support(),
    }
}

pub fn backend() -> &'static str {
    "secret-service"
}

fn lib() -> Result<&'static Lib, KeychainError> {
    Lib::get().ok_or(KeychainError::Unsupported)
}

pub fn set(key: &str, secret: &[u8], protection: Protection) -> Result<(), KeychainError> {
    if protection == Protection::UserPresence {
        return Err(KeychainError::Unsupported);
    }
    let lib = lib()?;
    let attrs = Attributes::new(lib, Some(key))?;
    let label = CString::new(format!("{}: {key}", crate::app_id()))
        .map_err(|_| KeychainError::InvalidKey)?;
    let mut text = crate::base64::encode(secret);
    let password =
        CString::new(std::mem::take(&mut text)).map_err(|_| KeychainError::InvalidKey)?;
    let mut error: *mut GError = std::ptr::null_mut();
    // SAFETY: every pointer is live for the call (the attributes table, the NUL-terminated label
    // and password), a null collection means the default one, and a null cancellable means none.
    let stored = unsafe {
        (lib.storev_sync)(
            schema(),
            attrs.table,
            std::ptr::null(),
            label.as_ptr(),
            password.as_ptr(),
            std::ptr::null_mut(),
            &mut error,
        )
    };
    // The base64 copy is a secret too; wipe it as the CString is dropped.
    let mut bytes = password.into_bytes();
    crate::wipe(&mut bytes);
    if stored == 0 {
        return Err(take_error(lib, error));
    }
    Ok(())
}

/// Look the item up; the Secret Service unlocks the keyring (with its dialog) when it must.
fn lookup(lib: &'static Lib, key: &str) -> Result<Vec<u8>, KeychainError> {
    let attrs = Attributes::new(lib, Some(key))?;
    let mut error: *mut GError = std::ptr::null_mut();
    // SAFETY: the table is live for the call; the returned string is ours to free with
    // `secret_password_free`, which wipes it first.
    unsafe {
        let text = (lib.lookupv_sync)(schema(), attrs.table, std::ptr::null_mut(), &mut error);
        if text.is_null() {
            return Err(if error.is_null() {
                KeychainError::NotFound
            } else {
                take_error(lib, error)
            });
        }
        let decoded = CStr::from_ptr(text)
            .to_str()
            .ok()
            .and_then(crate::base64::decode);
        (lib.password_free)(text);
        decoded.ok_or(KeychainError::Unreadable)
    }
}

pub fn get(key: &str) -> Result<Vec<u8>, KeychainError> {
    lookup(lib()?, key)
}

pub fn get_with_presence(key: String, _reason: String, on_done: OnDone) {
    // Nothing here needs the person, but the keyring may need unlocking, and that dialog must
    // not block the UI thread: the lookup runs on a thread of its own.
    let lib = match lib() {
        Ok(lib) => lib,
        Err(e) => {
            on_done(Err(e));
            return;
        }
    };
    let spawned = std::thread::Builder::new()
        .name("day-keychain".into())
        .spawn(move || on_done(lookup(lib, &key)));
    if let Err(e) = spawned {
        eprintln!("day-part-keychain: could not start the lookup thread: {e}");
    }
}

pub fn remove(key: &str) -> Result<(), KeychainError> {
    let lib = lib()?;
    let attrs = Attributes::new(lib, Some(key))?;
    let mut error: *mut GError = std::ptr::null_mut();
    // SAFETY: the table is live for the call.
    let cleared =
        unsafe { (lib.clearv_sync)(schema(), attrs.table, std::ptr::null_mut(), &mut error) };
    if cleared != 0 {
        return Ok(());
    }
    // FALSE with no error: nothing matched (or every match is in a locked collection, which the
    // service does not say); the item is not there to remove.
    Err(if error.is_null() {
        KeychainError::NotFound
    } else {
        take_error(lib, error)
    })
}

pub fn contains(key: &str) -> Result<bool, KeychainError> {
    Ok(entries()?.iter().any(|e| e.key == key))
}

pub fn entries() -> Result<Vec<Entry>, KeychainError> {
    let lib = lib()?;
    let attrs = Attributes::new(lib, None)?;
    let mut error: *mut GError = std::ptr::null_mut();
    // SAFETY: the table is live for the call; the returned list owns one reference per item,
    // released with `g_list_free_full(list, g_object_unref)`; each attributes table is a new
    // reference released with `g_hash_table_unref`.
    unsafe {
        let list = (lib.searchv_sync)(
            schema(),
            attrs.table,
            SEARCH_ALL,
            std::ptr::null_mut(),
            &mut error,
        );
        if list.is_null() {
            return if error.is_null() {
                Ok(Vec::new())
            } else {
                Err(take_error(lib, error))
            };
        }
        let mut out = Vec::new();
        let mut node = list;
        while !node.is_null() {
            let item = (*node).data;
            if !item.is_null() {
                let table = (lib.item_get_attributes)(item);
                if !table.is_null() {
                    let value = (lib.hash_table_lookup)(table, ATTR_KEY.as_ptr().cast());
                    if !value.is_null() {
                        let key = CStr::from_ptr(value.cast::<c_char>())
                            .to_string_lossy()
                            .into_owned();
                        out.push(Entry {
                            key,
                            protection: Protection::WhenUnlocked,
                        });
                    }
                    (lib.hash_table_unref)(table);
                }
            }
            node = (*node).next;
        }
        (lib.list_free_full)(list, lib.object_unref as *const c_void);
        Ok(out)
    }
}
