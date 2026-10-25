// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! A `StatusNotifierItem` server with its `com.canonical.dbusmenu` menu: the Linux status item
//! ("tray icon") that KDE Plasma, GNOME with the AppIndicator extension, Xfce, LXQt, Budgie,
//! Cinnamon and the wlroots bars (waybar, …) display.
//!
//! The server is toolkit-agnostic. It takes a neutral [`Item`] (id, title, tooltip, icon as
//! ARGB32 pixmaps and/or a theme icon name, status) and a neutral menu tree of [`MenuEntry`],
//! exports `/StatusNotifierItem` and `/MenuBar`, registers with the
//! `org.kde.StatusNotifierWatcher`, and re-registers whenever the watcher (re)appears. Clicks and
//! menu selections arrive as [`Event`]s through a callback on the connection's reader thread; a
//! toolkit backend forwards them to its UI thread.
//!
//! The item owns `org.kde.StatusNotifierItem-<pid>-<n>`, except inside Flatpak or Snap, where
//! the sandbox does not let an app own that name; there it registers by its unique bus name,
//! which every watcher accepts.
//!
//! One connection serves one item, because the object paths are fixed by the protocol; open a
//! further [`Connection`] for a second item.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use super::{
    BUS_INTERFACE, BUS_NAME, Connection, Error, Message, MethodCall, MethodError,
    NAME_FLAG_DO_NOT_QUEUE, NAME_REPLY_ALREADY_OWNER, NAME_REPLY_PRIMARY_OWNER,
    PROPERTIES_INTERFACE, SignalFilter, SubscriptionId, Value, introspection_xml, lock,
};

/// The item's object path.
pub const ITEM_PATH: &str = "/StatusNotifierItem";
/// The menu's object path.
pub const MENU_PATH: &str = "/MenuBar";
/// The item interface.
pub const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
/// The menu interface.
pub const MENU_INTERFACE: &str = "com.canonical.dbusmenu";
/// The watcher's well-known name, which is also its interface.
pub const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
/// The watcher's object path.
pub const WATCHER_PATH: &str = "/StatusNotifierWatcher";

/// Whether a `StatusNotifierWatcher` is on the bus, that is, whether a status item can show.
pub fn watcher_available(conn: &Connection) -> bool {
    conn.name_has_owner(WATCHER_NAME)
}

/// One icon image: `width × height` pixels of ARGB32, row by row. Each `u32` is
/// `0xAARRGGBB` (non-premultiplied); on the wire it goes out in network byte order (big-endian
/// A, R, G, B), as the specification requires whatever the host's byte order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pixmap {
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// `width × height` pixels, `0xAARRGGBB` each.
    pub argb: Vec<u32>,
}

impl Pixmap {
    /// Converts straight (non-premultiplied) RGBA8 bytes; `None` when the length is not
    /// `width × height × 4` or a dimension does not fit `i32`.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Pixmap> {
        let pixels = usize::try_from(u64::from(width) * u64::from(height)).ok()?;
        if rgba.len() != pixels.checked_mul(4)? {
            return None;
        }
        Some(Pixmap {
            width: i32::try_from(width).ok()?,
            height: i32::try_from(height).ok()?,
            argb: rgba
                .as_chunks::<4>()
                .0
                .iter()
                .map(|[r, g, b, a]| u32::from_be_bytes([*a, *r, *g, *b]))
                .collect(),
        })
    }

    /// The `(iiay)` struct, or `None` when the pixel count does not match the size.
    fn to_value(&self) -> Option<Value> {
        let expected = usize::try_from(self.width).ok()? * usize::try_from(self.height).ok()?;
        if expected != self.argb.len() {
            return None;
        }
        let mut bytes = Vec::with_capacity(expected * 4);
        for px in &self.argb {
            bytes.extend_from_slice(&px.to_be_bytes());
        }
        Some(Value::Struct(vec![
            Value::I32(self.width),
            Value::I32(self.height),
            Value::bytes(&bytes),
        ]))
    }
}

fn pixmaps_value(pixmaps: &[Pixmap]) -> Value {
    Value::array(
        "(iiay)",
        pixmaps.iter().filter_map(Pixmap::to_value).collect(),
    )
}

/// The item's status, which hosts use to show or hide it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Shown normally.
    #[default]
    Active,
    /// Present but unimportant; hosts may hide it in an overflow area.
    Passive,
    /// Asks for the user's attention.
    NeedsAttention,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Active => "Active",
            Status::Passive => "Passive",
            Status::NeedsAttention => "NeedsAttention",
        }
    }
}

/// What kind of item this is, which some hosts use to group items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    /// An application's own status (the usual choice).
    #[default]
    ApplicationStatus,
    /// A communication app (chat, mail).
    Communications,
    /// A system service.
    SystemServices,
    /// A hardware indicator (battery, network).
    Hardware,
}

impl Category {
    fn as_str(self) -> &'static str {
        match self {
            Category::ApplicationStatus => "ApplicationStatus",
            Category::Communications => "Communications",
            Category::SystemServices => "SystemServices",
            Category::Hardware => "Hardware",
        }
    }
}

/// The item's description.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    /// A stable identifier for the item, such as the app id.
    pub id: String,
    /// The item's title (shown by some hosts, and read by screen readers).
    pub title: String,
    /// The tooltip text; empty uses the title.
    pub tooltip: String,
    /// A freedesktop icon theme name; hosts prefer it over pixmaps when they find it.
    pub icon_name: String,
    /// Extra directory searched for `icon_name`, or empty.
    pub icon_theme_path: String,
    /// The icon at several sizes, for hosts without the theme icon.
    pub icon_pixmaps: Vec<Pixmap>,
    /// Active, passive or needs-attention.
    pub status: Status,
    /// The item's category.
    pub category: Category,
    /// Whether a primary click opens the menu instead of sending [`Event::Activate`].
    pub item_is_menu: bool,
}

/// A menu entry's checkmark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Check {
    /// Not checkable.
    #[default]
    None,
    /// Checkable and checked.
    On,
    /// Checkable and unchecked.
    Off,
}

/// One menu entry. Ids are chosen by the caller, must be unique within the menu, and must not be
/// 0 (the root). A separator ignores its label; an entry with children is a submenu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    /// The id reported in [`Event::MenuItem`].
    pub id: i32,
    /// The label, shown literally (underscores are escaped from dbusmenu's mnemonic syntax).
    pub label: String,
    /// Whether the entry can be chosen.
    pub enabled: bool,
    /// Whether the entry is shown.
    pub visible: bool,
    /// The checkmark state.
    pub check: Check,
    /// Whether the entry is a separator line.
    pub separator: bool,
    /// The submenu's entries.
    pub children: Vec<MenuEntry>,
}

impl MenuEntry {
    /// An enabled, visible, uncheckable entry.
    pub fn item(id: i32, label: impl Into<String>) -> Self {
        MenuEntry {
            id,
            label: label.into(),
            enabled: true,
            visible: true,
            check: Check::None,
            separator: false,
            children: Vec::new(),
        }
    }

    /// A separator.
    pub fn separator(id: i32) -> Self {
        MenuEntry {
            separator: true,
            ..MenuEntry::item(id, "")
        }
    }

    /// A submenu.
    pub fn submenu(id: i32, label: impl Into<String>, children: Vec<MenuEntry>) -> Self {
        MenuEntry {
            children,
            ..MenuEntry::item(id, label)
        }
    }

    /// The same entry with a checkmark state.
    pub fn with_check(mut self, check: Check) -> Self {
        self.check = check;
        self
    }

    /// The same entry with `enabled` set.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// What the user did with the item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Primary click at screen position `(x, y)`.
    Activate {
        /// Screen x.
        x: i32,
        /// Screen y.
        y: i32,
    },
    /// Middle click.
    SecondaryActivate {
        /// Screen x.
        x: i32,
        /// Screen y.
        y: i32,
    },
    /// The host asks the app to show its own context menu (hosts that use the exported menu
    /// never send this).
    ContextMenu {
        /// Screen x.
        x: i32,
        /// Screen y.
        y: i32,
    },
    /// Scroll over the item.
    Scroll {
        /// Steps scrolled.
        delta: i32,
        /// Horizontal rather than vertical.
        horizontal: bool,
    },
    /// A menu entry was chosen.
    MenuItem {
        /// The entry's id.
        id: i32,
    },
}

struct State {
    item: Item,
    menu: Vec<MenuEntry>,
    revision: u32,
}

type EventFn = dyn FnMut(Event) + Send;

struct Core {
    state: Mutex<State>,
    on_event: Mutex<Box<EventFn>>,
}

impl Core {
    fn deliver(&self, event: Event) {
        let mut callback = lock(&self.on_event);
        (callback)(event);
    }
}

/// A live status item. Dropping it removes the item from the bus.
pub struct StatusItem {
    conn: Connection,
    core: Arc<Core>,
    service: String,
    owns_name: bool,
    subscription: Option<SubscriptionId>,
    match_rule: String,
}

static NEXT_ITEM: AtomicU32 = AtomicU32::new(1);

fn sandboxed() -> bool {
    Path::new("/.flatpak-info").exists() || std::env::var_os("SNAP").is_some()
}

impl StatusItem {
    /// Exports the item and its menu on `conn` and registers with the watcher if one is
    /// running (and again whenever one starts). `on_event` runs on the connection's reader
    /// thread, so it should hand the event on rather than block.
    pub fn new<F>(
        conn: &Connection,
        item: Item,
        menu: Vec<MenuEntry>,
        on_event: F,
    ) -> Result<StatusItem, Error>
    where
        F: FnMut(Event) + Send + 'static,
    {
        let core = Arc::new(Core {
            state: Mutex::new(State {
                item,
                menu,
                revision: 1,
            }),
            on_event: Mutex::new(Box::new(on_event)),
        });
        let item_core = core.clone();
        conn.export(ITEM_PATH, move |call| item_call(&item_core, call))?;
        let menu_core = core.clone();
        if let Err(e) = conn.export(MENU_PATH, move |call| menu_call(&menu_core, call)) {
            // MENU_PATH belongs to someone else; only ITEM_PATH is ours to remove.
            conn.unexport(ITEM_PATH);
            return Err(e);
        }
        // From here on, Drop undoes whatever succeeded.
        let mut this = StatusItem {
            conn: conn.clone(),
            core,
            service: conn.unique_name().to_owned(),
            owns_name: false,
            subscription: None,
            match_rule: String::new(),
        };

        if !sandboxed() {
            let n = NEXT_ITEM.fetch_add(1, Ordering::Relaxed);
            let name = format!("org.kde.StatusNotifierItem-{}-{n}", std::process::id());
            match conn.request_name(&name, NAME_FLAG_DO_NOT_QUEUE)? {
                NAME_REPLY_PRIMARY_OWNER | NAME_REPLY_ALREADY_OWNER => {
                    this.service = name;
                    this.owns_name = true;
                }
                code => {
                    return Err(Error::Protocol(format!(
                        "could not own {name} (RequestName answered {code})"
                    )));
                }
            }
        }

        // Subscribe before looking for the watcher, so one that starts in between is not
        // missed.
        let rule = format!(
            "type='signal',sender='{BUS_NAME}',interface='{BUS_INTERFACE}',\
             member='NameOwnerChanged',arg0='{WATCHER_NAME}'"
        );
        conn.add_match(&rule)?;
        this.match_rule = rule;
        let service = this.service.clone();
        this.subscription = Some(
            conn.on_signal(
                SignalFilter::new()
                    .sender(BUS_NAME)
                    .interface(BUS_INTERFACE)
                    .member("NameOwnerChanged"),
                move |conn, msg| watcher_changed(conn, msg, &service),
            ),
        );
        if watcher_available(conn) {
            conn.call(
                WATCHER_NAME,
                WATCHER_PATH,
                WATCHER_NAME,
                "RegisterStatusNotifierItem",
                &[Value::str(&this.service)],
            )?;
        }
        Ok(this)
    }

    /// The bus name the item registered under: its own well-known name, or the connection's
    /// unique name inside a sandbox.
    pub fn service(&self) -> &str {
        &self.service
    }

    fn emit(&self, member: &str, args: &[Value]) -> Result<(), Error> {
        self.conn
            .emit_signal(ITEM_PATH, ITEM_INTERFACE, member, args)
    }

    /// Replaces the menu and tells hosts to fetch it again.
    pub fn set_menu(&self, menu: Vec<MenuEntry>) -> Result<(), Error> {
        let revision = {
            let mut state = lock(&self.core.state);
            state.menu = menu;
            state.revision = state.revision.wrapping_add(1);
            state.revision
        };
        self.conn.emit_signal(
            MENU_PATH,
            MENU_INTERFACE,
            "LayoutUpdated",
            &[Value::U32(revision), Value::I32(0)],
        )
    }

    /// Replaces the icon (theme name and pixmaps).
    pub fn set_icon(&self, icon_name: &str, pixmaps: Vec<Pixmap>) -> Result<(), Error> {
        {
            let mut state = lock(&self.core.state);
            state.item.icon_name = icon_name.to_owned();
            state.item.icon_pixmaps = pixmaps;
        }
        self.emit("NewIcon", &[])
    }

    /// Replaces the title.
    pub fn set_title(&self, title: &str) -> Result<(), Error> {
        lock(&self.core.state).item.title = title.to_owned();
        self.emit("NewTitle", &[])
    }

    /// Replaces the tooltip text.
    pub fn set_tooltip(&self, tooltip: &str) -> Result<(), Error> {
        lock(&self.core.state).item.tooltip = tooltip.to_owned();
        self.emit("NewToolTip", &[])
    }

    /// Changes the status.
    pub fn set_status(&self, status: Status) -> Result<(), Error> {
        lock(&self.core.state).item.status = status;
        self.emit("NewStatus", &[Value::str(status.as_str())])
    }
}

impl Drop for StatusItem {
    fn drop(&mut self) {
        // Nothing here waits for the bus: a drop may run on the reader thread.
        self.conn.unexport(ITEM_PATH);
        self.conn.unexport(MENU_PATH);
        if let Some(id) = self.subscription.take() {
            self.conn.remove_signal_handler(id);
        }
        if !self.match_rule.is_empty() {
            let _ = self.conn.remove_match(&self.match_rule);
        }
        if self.owns_name {
            let _ = self.conn.release_name(&self.service);
        }
    }
}

fn watcher_changed(conn: &Connection, msg: &Message, service: &str) {
    if let [Value::Str(name), _, Value::Str(new_owner)] = msg.body.as_slice()
        && name == WATCHER_NAME
        && !new_owner.is_empty()
    {
        // The reader thread cannot wait for a reply; registration needs none.
        let _ = conn.call_no_reply(
            WATCHER_NAME,
            WATCHER_PATH,
            WATCHER_NAME,
            "RegisterStatusNotifierItem",
            &[Value::str(service)],
        );
    }
}

// ---------------------------------------------------------------------------
// org.kde.StatusNotifierItem
// ---------------------------------------------------------------------------

fn item_property(item: &Item, name: &str) -> Option<Value> {
    let empty_pixmaps = || pixmaps_value(&[]);
    Some(match name {
        "Category" => Value::str(item.category.as_str()),
        "Id" => Value::str(&item.id),
        "Title" => Value::str(&item.title),
        "Status" => Value::str(item.status.as_str()),
        "WindowId" => Value::I32(0),
        "IconName" => Value::str(&item.icon_name),
        "IconThemePath" => Value::str(&item.icon_theme_path),
        "IconPixmap" => pixmaps_value(&item.icon_pixmaps),
        "OverlayIconName" | "AttentionIconName" | "AttentionMovieName" => Value::str(""),
        "OverlayIconPixmap" | "AttentionIconPixmap" => empty_pixmaps(),
        "ToolTip" => Value::Struct(vec![
            Value::str(""),
            empty_pixmaps(),
            Value::str(if item.tooltip.is_empty() {
                &item.title
            } else {
                &item.tooltip
            }),
            Value::str(""),
        ]),
        "ItemIsMenu" => Value::Bool(item.item_is_menu),
        "Menu" => Value::path(MENU_PATH),
        _ => return None,
    })
}

const ITEM_PROPERTIES: &[&str] = &[
    "Category",
    "Id",
    "Title",
    "Status",
    "WindowId",
    "IconName",
    "IconThemePath",
    "IconPixmap",
    "OverlayIconName",
    "OverlayIconPixmap",
    "AttentionIconName",
    "AttentionIconPixmap",
    "AttentionMovieName",
    "ToolTip",
    "ItemIsMenu",
    "Menu",
];

const ITEM_XML: &str = r#" <interface name="org.kde.StatusNotifierItem">
  <property name="Category" type="s" access="read"/>
  <property name="Id" type="s" access="read"/>
  <property name="Title" type="s" access="read"/>
  <property name="Status" type="s" access="read"/>
  <property name="WindowId" type="i" access="read"/>
  <property name="IconName" type="s" access="read"/>
  <property name="IconThemePath" type="s" access="read"/>
  <property name="IconPixmap" type="a(iiay)" access="read"/>
  <property name="OverlayIconName" type="s" access="read"/>
  <property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
  <property name="AttentionIconName" type="s" access="read"/>
  <property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
  <property name="AttentionMovieName" type="s" access="read"/>
  <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
  <property name="ItemIsMenu" type="b" access="read"/>
  <property name="Menu" type="o" access="read"/>
  <method name="ContextMenu"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
  <method name="Activate"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
  <method name="SecondaryActivate"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
  <method name="Scroll"><arg name="delta" type="i" direction="in"/><arg name="orientation" type="s" direction="in"/></method>
  <signal name="NewTitle"/>
  <signal name="NewIcon"/>
  <signal name="NewAttentionIcon"/>
  <signal name="NewOverlayIcon"/>
  <signal name="NewToolTip"/>
  <signal name="NewStatus"><arg name="status" type="s"/></signal>
 </interface>
 <interface name="org.freedesktop.DBus.Properties">
  <method name="Get"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
  <method name="GetAll"><arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method>
 </interface>
"#;

/// The two `i` arguments of `Activate`-style methods.
fn point(args: &[Value]) -> Result<(i32, i32), MethodError> {
    match args {
        [Value::I32(x), Value::I32(y)] => Ok((*x, *y)),
        _ => Err(MethodError::invalid_args("expected (ii)")),
    }
}

fn string_arg(args: &[Value], at: usize) -> Result<&str, MethodError> {
    match args.get(at) {
        Some(Value::Str(s)) => Ok(s),
        _ => Err(MethodError::invalid_args(format!(
            "argument {at} must be a string"
        ))),
    }
}

/// Answers `Properties.Get`/`GetAll`/`Set` for one interface's read-only properties.
fn properties(
    call: &MethodCall,
    interface: &str,
    names: &[&str],
    get: impl Fn(&str) -> Option<Value>,
) -> Result<Vec<Value>, MethodError> {
    match call.member.as_str() {
        "Get" => {
            let (iface, name) = (string_arg(&call.args, 0)?, string_arg(&call.args, 1)?);
            match get(name) {
                Some(v) if iface == interface || iface.is_empty() => Ok(vec![Value::variant(v)]),
                _ => Err(MethodError::unknown_property(name)),
            }
        }
        "GetAll" => {
            let iface = string_arg(&call.args, 0)?;
            let all = if iface == interface || iface.is_empty() {
                names
                    .iter()
                    .filter_map(|n| get(n).map(|v| (*n, v)))
                    .collect()
            } else {
                Vec::new()
            };
            Ok(vec![Value::props(all)])
        }
        "Set" => Err(MethodError::new(
            "org.freedesktop.DBus.Error.PropertyReadOnly",
            "properties are read-only",
        )),
        _ => Err(MethodError::unknown_method(call)),
    }
}

fn item_call(core: &Core, call: &MethodCall) -> Result<Vec<Value>, MethodError> {
    match call.interface.as_deref() {
        Some(PROPERTIES_INTERFACE) => {
            let item = lock(&core.state).item.clone();
            return properties(call, ITEM_INTERFACE, ITEM_PROPERTIES, |n| {
                item_property(&item, n)
            });
        }
        Some(super::INTROSPECTABLE_INTERFACE) | None if call.member == "Introspect" => {
            return Ok(vec![Value::Str(introspection_xml(ITEM_XML, &[]))]);
        }
        Some(ITEM_INTERFACE) | None => {}
        Some(_) => return Err(MethodError::unknown_method(call)),
    }
    let event = match call.member.as_str() {
        "Activate" => {
            let (x, y) = point(&call.args)?;
            Event::Activate { x, y }
        }
        "SecondaryActivate" => {
            let (x, y) = point(&call.args)?;
            Event::SecondaryActivate { x, y }
        }
        "ContextMenu" => {
            let (x, y) = point(&call.args)?;
            Event::ContextMenu { x, y }
        }
        "Scroll" => match call.args.as_slice() {
            [Value::I32(delta), Value::Str(orientation)] => Event::Scroll {
                delta: *delta,
                horizontal: orientation.eq_ignore_ascii_case("horizontal"),
            },
            _ => return Err(MethodError::invalid_args("expected (is)")),
        },
        // Newer hosts hand over an activation token before Activate; nothing to do with it.
        "ProvideXdgActivationToken" => return Ok(Vec::new()),
        _ => return Err(MethodError::unknown_method(call)),
    };
    core.deliver(event);
    Ok(Vec::new())
}

// ---------------------------------------------------------------------------
// com.canonical.dbusmenu
// ---------------------------------------------------------------------------

const MENU_XML: &str = r#" <interface name="com.canonical.dbusmenu">
  <property name="Version" type="u" access="read"/>
  <property name="TextDirection" type="s" access="read"/>
  <property name="Status" type="s" access="read"/>
  <property name="IconThemePath" type="as" access="read"/>
  <method name="GetLayout"><arg type="i" name="parentId" direction="in"/><arg type="i" name="recursionDepth" direction="in"/><arg type="as" name="propertyNames" direction="in"/><arg type="u" name="revision" direction="out"/><arg type="(ia{sv}av)" name="layout" direction="out"/></method>
  <method name="GetGroupProperties"><arg type="ai" name="ids" direction="in"/><arg type="as" name="propertyNames" direction="in"/><arg type="a(ia{sv})" name="properties" direction="out"/></method>
  <method name="GetProperty"><arg type="i" name="id" direction="in"/><arg type="s" name="name" direction="in"/><arg type="v" name="value" direction="out"/></method>
  <method name="Event"><arg type="i" name="id" direction="in"/><arg type="s" name="eventId" direction="in"/><arg type="v" name="data" direction="in"/><arg type="u" name="timestamp" direction="in"/></method>
  <method name="EventGroup"><arg type="a(isvu)" name="events" direction="in"/><arg type="ai" name="idErrors" direction="out"/></method>
  <method name="AboutToShow"><arg type="i" name="id" direction="in"/><arg type="b" name="needUpdate" direction="out"/></method>
  <method name="AboutToShowGroup"><arg type="ai" name="ids" direction="in"/><arg type="ai" name="updatesNeeded" direction="out"/><arg type="ai" name="idErrors" direction="out"/></method>
  <signal name="ItemsPropertiesUpdated"><arg type="a(ia{sv})" name="updatedProps"/><arg type="a(ias)" name="removedProps"/></signal>
  <signal name="LayoutUpdated"><arg type="u" name="revision"/><arg type="i" name="parent"/></signal>
  <signal name="ItemActivationRequested"><arg type="i" name="id"/><arg type="u" name="timestamp"/></signal>
 </interface>
 <interface name="org.freedesktop.DBus.Properties">
  <method name="Get"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
  <method name="GetAll"><arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method>
 </interface>
"#;

const MENU_PROPERTIES: &[&str] = &["Version", "TextDirection", "Status", "IconThemePath"];

fn menu_property(name: &str) -> Option<Value> {
    Some(match name {
        "Version" => Value::U32(3),
        "TextDirection" => Value::str("ltr"),
        "Status" => Value::str("normal"),
        "IconThemePath" => Value::strings::<&str>(&[]),
        _ => return None,
    })
}

fn find(entries: &[MenuEntry], id: i32) -> Option<&MenuEntry> {
    entries.iter().find_map(|e| {
        if e.id == id {
            Some(e)
        } else {
            find(&e.children, id)
        }
    })
}

fn all_entries(entries: &[MenuEntry], out: &mut Vec<(i32, Vec<(&'static str, Value)>)>) {
    for e in entries {
        out.push((e.id, entry_props(e)));
        all_entries(&e.children, out);
    }
}

/// dbusmenu treats `_` as the mnemonic marker; a literal one is doubled.
fn escape_label(label: &str) -> String {
    label.replace('_', "__")
}

/// An entry's properties, all of them; the caller filters by requested names.
fn entry_props(e: &MenuEntry) -> Vec<(&'static str, Value)> {
    let mut props = Vec::new();
    if e.separator {
        props.push(("type", Value::str("separator")));
    } else {
        props.push(("label", Value::str(escape_label(&e.label))));
        props.push(("enabled", Value::Bool(e.enabled)));
    }
    props.push(("visible", Value::Bool(e.visible)));
    if e.check != Check::None && !e.separator {
        props.push(("toggle-type", Value::str("checkmark")));
        props.push(("toggle-state", Value::I32(i32::from(e.check == Check::On))));
    }
    if !e.children.is_empty() {
        props.push(("children-display", Value::str("submenu")));
    }
    props
}

fn root_props() -> Vec<(&'static str, Value)> {
    vec![("children-display", Value::str("submenu"))]
}

fn filtered(props: Vec<(&'static str, Value)>, names: &[String]) -> Value {
    Value::props(
        props
            .into_iter()
            .filter(|(k, _)| names.is_empty() || names.iter().any(|n| n == k)),
    )
}

fn layout(
    id: i32,
    props: Vec<(&'static str, Value)>,
    children: &[MenuEntry],
    depth: i32,
    names: &[String],
) -> Value {
    let kids = if depth == 0 {
        Vec::new()
    } else {
        let next = if depth < 0 { -1 } else { depth - 1 };
        children
            .iter()
            .map(|c| Value::variant(layout(c.id, entry_props(c), &c.children, next, names)))
            .collect()
    };
    Value::Struct(vec![
        Value::I32(id),
        filtered(props, names),
        Value::array("v", kids),
    ])
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

fn int_list(v: Option<&Value>) -> Vec<i32> {
    v.and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|n| match n {
            Value::I32(n) => Some(*n),
            _ => None,
        })
        .collect()
}

/// Handles one `Event`; `false` when the id is unknown.
fn menu_event(core: &Core, id: i32, event_id: &str) -> bool {
    let clickable = {
        let state = lock(&core.state);
        if id == 0 {
            Some(false)
        } else {
            find(&state.menu, id).map(|e| e.enabled && !e.separator)
        }
    };
    match clickable {
        None => false,
        Some(clickable) => {
            if clickable && event_id == "clicked" {
                core.deliver(Event::MenuItem { id });
            }
            true
        }
    }
}

fn menu_call(core: &Core, call: &MethodCall) -> Result<Vec<Value>, MethodError> {
    match call.interface.as_deref() {
        Some(PROPERTIES_INTERFACE) => {
            return properties(call, MENU_INTERFACE, MENU_PROPERTIES, menu_property);
        }
        Some(super::INTROSPECTABLE_INTERFACE) | None if call.member == "Introspect" => {
            return Ok(vec![Value::Str(introspection_xml(MENU_XML, &[]))]);
        }
        Some(MENU_INTERFACE) | None => {}
        Some(_) => return Err(MethodError::unknown_method(call)),
    }
    let args = call.args.as_slice();
    match call.member.as_str() {
        "GetLayout" => {
            let [Value::I32(parent), Value::I32(depth), names] = args else {
                return Err(MethodError::invalid_args("expected (iias)"));
            };
            let names = string_list(Some(names));
            let state = lock(&core.state);
            let node = if *parent == 0 {
                layout(0, root_props(), &state.menu, *depth, &names)
            } else {
                let entry = find(&state.menu, *parent)
                    .ok_or_else(|| MethodError::invalid_args(format!("no menu entry {parent}")))?;
                layout(
                    entry.id,
                    entry_props(entry),
                    &entry.children,
                    *depth,
                    &names,
                )
            };
            Ok(vec![Value::U32(state.revision), node])
        }
        "GetGroupProperties" => {
            let ids = int_list(args.first());
            let names = string_list(args.get(1));
            let state = lock(&core.state);
            let mut rows = Vec::new();
            if ids.is_empty() {
                all_entries(&state.menu, &mut rows);
            } else {
                for id in ids {
                    if id == 0 {
                        rows.push((0, root_props()));
                    } else if let Some(e) = find(&state.menu, id) {
                        rows.push((id, entry_props(e)));
                    }
                }
            }
            let rows = rows
                .into_iter()
                .map(|(id, props)| Value::Struct(vec![Value::I32(id), filtered(props, &names)]))
                .collect();
            Ok(vec![Value::array("(ia{sv})", rows)])
        }
        "GetProperty" => {
            let [Value::I32(id), Value::Str(name)] = args else {
                return Err(MethodError::invalid_args("expected (is)"));
            };
            let state = lock(&core.state);
            let props = if *id == 0 {
                root_props()
            } else {
                find(&state.menu, *id)
                    .map(entry_props)
                    .ok_or_else(|| MethodError::invalid_args(format!("no menu entry {id}")))?
            };
            props
                .into_iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| vec![Value::variant(v)])
                .ok_or_else(|| MethodError::unknown_property(name))
        }
        "Event" => {
            let [Value::I32(id), Value::Str(event_id), ..] = args else {
                return Err(MethodError::invalid_args("expected (isvu)"));
            };
            if menu_event(core, *id, event_id) {
                Ok(Vec::new())
            } else {
                Err(MethodError::invalid_args(format!("no menu entry {id}")))
            }
        }
        "EventGroup" => {
            let events = args.first().and_then(Value::as_array).unwrap_or_default();
            let mut errors = Vec::new();
            for event in events {
                if let Some([Value::I32(id), Value::Str(event_id), ..]) = event.as_struct()
                    && !menu_event(core, *id, event_id)
                {
                    errors.push(Value::I32(*id));
                }
            }
            Ok(vec![Value::array("i", errors)])
        }
        "AboutToShow" => Ok(vec![Value::Bool(false)]),
        "AboutToShowGroup" => {
            let state = lock(&core.state);
            let errors = int_list(args.first())
                .into_iter()
                .filter(|id| *id != 0 && find(&state.menu, *id).is_none())
                .map(Value::I32)
                .collect();
            Ok(vec![
                Value::array("i", Vec::new()),
                Value::array("i", errors),
            ])
        }
        _ => Err(MethodError::unknown_method(call)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixmaps_go_out_big_endian_argb() {
        let px = Pixmap::from_rgba(1, 1, &[0x10, 0x20, 0x30, 0xff]).expect("pixmap");
        assert_eq!(px.argb, vec![0xff10_2030]);
        let v = px.to_value().expect("value");
        assert_eq!(
            v,
            Value::Struct(vec![
                Value::I32(1),
                Value::I32(1),
                Value::bytes(&[0xff, 0x10, 0x20, 0x30])
            ])
        );
        assert_eq!(pixmaps_value(&[px]).signature(), "a(iiay)");
        assert!(Pixmap::from_rgba(2, 2, &[0; 4]).is_none());
        let bad = Pixmap {
            width: 2,
            height: 2,
            argb: vec![0],
        };
        assert!(bad.to_value().is_none());
    }

    #[test]
    fn tooltip_and_properties_have_spec_signatures() {
        let item = Item {
            title: "T".into(),
            ..Item::default()
        };
        for name in ITEM_PROPERTIES {
            assert!(item_property(&item, name).is_some(), "{name}");
        }
        let tip = item_property(&item, "ToolTip").expect("tooltip");
        assert_eq!(tip.signature(), "(sa(iiay)ss)");
        assert_eq!(tip.as_struct().map(|f| f[2].clone()), Some(Value::str("T")));
        assert_eq!(item_property(&item, "Menu"), Some(Value::path(MENU_PATH)));
        // Everything marshals.
        let all: Vec<Value> = ITEM_PROPERTIES
            .iter()
            .filter_map(|n| item_property(&item, n))
            .collect();
        assert!(super::super::marshal(&all).is_ok());
    }

    #[test]
    fn layout_respects_depth_and_names() {
        let menu = vec![
            MenuEntry::item(1, "Open_File").with_check(Check::On),
            MenuEntry::separator(2),
            MenuEntry::submenu(3, "More", vec![MenuEntry::item(4, "Deep")]),
        ];
        let full = layout(0, root_props(), &menu, -1, &[]);
        assert_eq!(full.signature(), "(ia{sv}av)");
        let kids = full.as_struct().expect("struct")[2].as_array().expect("av");
        assert_eq!(kids.len(), 3);
        let first = kids[0].as_struct().expect("child");
        assert_eq!(first[1].dict_get("label"), Some(&Value::str("Open__File")));
        assert_eq!(first[1].dict_get("toggle-state"), Some(&Value::I32(1)));
        let sub = kids[2].as_struct().expect("submenu");
        assert_eq!(sub[2].as_array().map(<[Value]>::len), Some(1));

        let shallow = layout(0, root_props(), &menu, 1, &["label".into()]);
        let kids = shallow.as_struct().expect("struct")[2]
            .as_array()
            .expect("av");
        let sub = kids[2].as_struct().expect("submenu");
        assert_eq!(sub[2].as_array().map(<[Value]>::len), Some(0));
        assert_eq!(sub[1].as_array().map(<[Value]>::len), Some(1));
        assert!(super::super::marshal(&[full, shallow]).is_ok());
    }
}
