app-title = Day Showcase
counter-value = { $count ->
    [one] { $count } click
   *[other] { $count } clicks
}
decrement = −
increment = +
name-placeholder = Your name
greeting = Hello, { $name }!
volume-label = Volume
progress-label = Progress
busy-label = Busy
flavor-label = Flavor
history-title = History
history-entry = count became { $value }
nav-controls = Controls
nav-menus = Menus
nav-text = Text
nav-battery = Battery
nav-sensors = Sensors
nav-clipboard = Clipboard
nav-network = Network
nav-media = Media
nav-pickers = Pickers
nav-compose = Compose
nav-files = Files
nav-tabs = Tabs
nav-stack = Stack
nav-list = List
nav-webview = Web View
nav-lottie = Lottie
nav-about = About

shapes-kinds = Kinds
shapes-transform = Transform
shapes-angle = Angle

picker-shared-caption = All three stylings are bound to the same selection signal — change one and the others follow.
picker-segmented = Segmented
picker-menu = Menu
picker-inline = Inline

compose-caption = Pure-composition pieces — no native code, no cargo features, every backend for free.
compose-rating-label = Star rating
compose-rating-count = Stars selected:
compose-rating-placeholder = 1–5
compose-card-title = Reusable surface
compose-card-body = Padding + background + rounded corners, applied as a Modifier.
compose-plain-btn = Plain
compose-styled-btn = Filled
compose-env-value = Tinted by the provided accent
list-add = Add 100
list-caption = { $count } rows — only the visible cells are built

webview-url-hint = Enter a URL
webview-go = Go
webview-back = Back
webview-forward = Forward
webview-stop = Stop
webview-reload = Reload

lottie-caption = A native Lottie animation, bundled as JSON (lottie-ios / lottie-android)
lottie-speed = Speed
stack-root-body = A genuine push/pop stack. Its path is an app-owned signal.
stack-push = Push a detail
stack-detail-title = Level { $depth }
stack-detail-body = Pushed onto the path. The native back button writes the pop back.
stack-item-title = Item { $id }
stack-link-42 = Open item-42 with a hint (absolute route)
stack-param-hint = Opened with hint: {$hint}
tab-one = Overview
tab-two = Details
tab-three = Settings
tab-one-body = The overview tab. Each tab keeps its own state.
tab-two-body = The details tab, selected by its route key.
tab-three-body = The settings tab. Deep links and dayscript select tabs by key.
about-text = A native cross-platform app built with day.
nav-modals = Modals
modal-alert = Show alert
modal-confirm = Confirm
modal-delete = Delete…
modal-sheet = Pick flavor
modal-prompt = Enter name
alert-title = Notice
alert-body = Your changes have been saved.
ok = OK
confirm-title = Quit?
confirm-body = Are you sure you want to quit?
delete-title = Delete item?
delete-body = This cannot be undone.
delete = Delete
flavor-title = Choose a flavor
cancel = Cancel
vanilla = vanilla
pistachio = pistachio

# Files playground (docs/files.md)
files-caption = Native open/save file pickers. Open reads a text file into the editor; Save writes it back out.
files-placeholder = Type something to save…
files-open = Open File…
files-save = Save File…
files-opened = Opened { $name }

# Battery playground (docs/battery.md)
battery-refresh = Read Device Battery
battery-level = Level
battery-charging = Charging
battery-reading = Battery: { $percent } · { $state }
battery-reading-none = Battery: no battery API on this platform

# Sensors playground (docs/sensors.md)
sensors-refresh = Read Sensors
sensor-accelerometer = Accelerometer
sensor-gyroscope = Gyroscope
sensor-magnetometer = Magnetometer
sensor-reading = x { $x } · y { $y } · z { $z } { $unit }
sensor-waiting = waiting for first sample…
sensor-unavailable = unavailable on this device

# Clipboard playground (docs/clipboard.md)
clipboard-caption = The day-part-clipboard part reads and writes the system clipboard natively.
clipboard-placeholder = Type something to copy
clipboard-copy = Copy
clipboard-paste = Paste
clipboard-idle = Clipboard untouched
clipboard-copied = Copied to the system clipboard
clipboard-copy-failed = Copy failed (no clipboard API here)
clipboard-pasted = Pasted from the system clipboard
clipboard-empty = Clipboard is empty (or unreadable in the background)

# Network playground (docs/network.md)
network-refresh = Read Network
network-reading-online = Online · { $kind } · metered: { $expensive }
network-reading-offline = Offline
network-reading-none = No connectivity API on this platform

# Media playground (docs/media.md)
media-play = Play
media-pause = Pause
media-load = Load

# Text playground (typography)
text-caption = Semantic styles map to the platform's native text styles and accessibility text scaling.
text-styles-header = Styles
text-weights-header = Weights
text-styling-header = Bold & italic
text-colors-header = Color
text-custom-header = Custom sizes
text-custom-note = Font.System(pt) — still scaled by the accessibility text size (Dynamic Type / font scale).
text-fonts-header = Bundled fonts
text-fonts-note = Font.Custom("Family", pt) — files from the app's fonts/ directory, bundled by day build and resolved by family name on every platform.

# Menus playground
menus-caption = Native menus — the app menu bar and per-piece context menus — with nested submenus, keyboard shortcuts, and standard Edit commands.
menus-last = Last action:
menus-lifecycle = Last lifecycle phase:
menus-context-hint = Context menu
menus-target = Right-click here (long-press on mobile) for a context menu
menus-shortcut-hint = Keyboard shortcuts (⌘/Ctrl + key) are shown in the menu bar and work while the app is focused — e.g. New (N), Save (S), Reload (R), Save As (⇧S).

# --- day-part-haptics ---
nav-haptics = Haptics
haptics-supported-yes = Haptic engine available on this platform
haptics-supported-no = No haptic engine on this platform (buttons are silent)
haptics-light = Light
haptics-medium = Medium
haptics-heavy = Heavy
haptics-success = Success
haptics-warning = Warning
haptics-error = Error
haptics-selection = Selection
haptics-last = Last played
haptics-none = Nothing played yet
haptics-last-played = Played: { $style }

# --- day-part-prefs ---
nav-prefs = Preferences
prefs-caption = Persist a string across launches with day-part-prefs.
prefs-placeholder = Value to remember
prefs-save = Save
prefs-load = Load
prefs-clear = Clear
prefs-idle = Type a value, then Save.
prefs-empty = (nothing stored)
prefs-saved = Saved.
prefs-save-failed = Save failed.
prefs-loaded = Loaded from storage.
prefs-missing = Nothing stored yet.
prefs-cleared = Cleared.
prefs-value-label = Stored value:

# --- bundled resources (§18.3) ---
nav-resources = Resources
resources-caption = An image loaded by name from a bundled resource, plus random-access reads of embedded data.
resources-numbers = numbers.bin: { $len } bytes, byte[100] = { $byte }
resources-greeting = greeting.txt: { $text }

# --- day-part-deviceinfo ---
nav-deviceinfo = Device Info
deviceinfo-model = Model: {$value}
deviceinfo-system = System: {$name} {$version}
deviceinfo-simulator = Simulator: {$value}
deviceinfo-yes = yes
deviceinfo-no = no
deviceinfo-refresh = Refresh

# --- day-piece-activity ---
activity-animating = Animating
activity-on = Spinning
activity-off = Stopped

# --- day-piece-searchfield ---
nav-search = Search
search-placeholder = Search fruit…
search-clear = Clear

# --- day-piece-map ---
nav-map = Map
map-caption = A native MKMapView — Apple platforms only. Tap a preset to recenter the map live.
map-sf = San Francisco
map-nyc = New York

# — tweaks page (docs/tweaks.md) —
nav-tweaks = Tweaks
tweaks-intro = Packaged tweaks configure the native widget behind a built-in piece, per toolkit. On toolkits a tweak doesn't cover, it is a no-op — the pieces below simply look stock.
tweaks-stock = Stock
tweaks-tweaked = Tweaked
tweaks-bezel-title = Button bezel
tweaks-bezel-caption = day-tweak-button-bezel — AppKit only: NSBezelStyle constants on the real NSButton.
tweaks-selectable-title = Selectable label
tweaks-selectable-caption = day-tweak-label-selectable — AppKit, GTK, Android: the platform's own text selection on a stock label.
tweaks-selectable-text = This label's text can be selected and copied — try it.
tweaks-ticks-title = Slider tick marks
tweaks-ticks-caption = day-tweak-slider-tickmarks — AppKit, GTK, Android, Qt, WinUI, ArkUI: native ticks, snapping where the platform supports it. The tweaked slider snaps; the stock one glides.
tweaks-ref-title = NativeRef liveness
tweaks-ref-caption = A NativeRef reaches the tweaked slider after mount; unmount it and the ref clears instead of dangling.
tweaks-ref-live = ref: live
tweaks-ref-cleared = ref: cleared

# — merged section pages (design overhaul) —
nav-canvas = Canvas & shapes
nav-system = Device & sensors
nav-services = Platform services
controls-caption = Two-way bindings: every control is a projection of an app-owned signal.
controls-basics = Basics
controls-feedback = Feedback
canvas-caption = Shapes, transforms, gestures, and composition-tier widgets — all drawn through the canvas.
canvas-gauge = Canvas gauge
shapes-interact-hint = Drag the slider to rotate, tap the circle to recolor, drag the purple square to move it.
system-caption = The headless device-state parts: battery, connectivity, motion sensors, and device identity.
services-caption = The headless "do something with the OS" parts: clipboard, preferences, haptics, and file pickers.
subscribe-label = Subscribe
