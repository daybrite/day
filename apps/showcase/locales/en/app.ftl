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
subscribe-a11y = Subscribe to updates
flavor-label = Flavor
history-title = History
history-entry = count became { $value }
nav-controls = Controls
nav-menus = Menus
nav-text = Text
nav-gauge = Gauge
nav-battery = Battery
nav-sensors = Sensors
nav-clipboard = Clipboard
nav-network = Network
nav-media = Media
nav-shapes = Shapes
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
shapes-tap = Tap to recolor
shapes-drag = Drag to move

picker-segmented = Segmented
picker-menu = Menu
picker-inline = Inline

compose-caption = Pure-composition pieces — no native code, no cargo features, every backend for free.
compose-rating-label = Star rating
compose-rating-count = Stars selected:
compose-rating-placeholder = 1–5
compose-card-label = Card modifier
compose-card-title = Reusable surface
compose-card-body = Padding + background + rounded corners, applied as a Modifier.
compose-badge-label = Badge overlay
compose-buttons-label = Button styles
compose-plain-btn = Plain
compose-styled-btn = Filled
compose-env-label = Ambient environment
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
battery-caption = The day-part-battery part reads the device battery natively; the canvas draws it.
battery-refresh = Read Device Battery
battery-preview = Preview
battery-level = Level
battery-charging = Charging
battery-reading = Battery: { $percent } · { $state }
battery-reading-none = Battery: no battery API on this platform

# Sensors playground (docs/sensors.md)
sensors-caption = The day-part-sensors part polls the device's motion sensors natively.
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
network-caption = The day-part-network part reads the device's connectivity snapshot natively.
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
haptics-caption = The day-part-haptics part plays native haptic feedback — each button fires one pattern.
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
deviceinfo-caption = Device identity read through the native platform API (headless day-part-deviceinfo).
deviceinfo-model = Model: {$value}
deviceinfo-system = System: {$name} {$version}
deviceinfo-simulator = Simulator: {$value}
deviceinfo-yes = yes
deviceinfo-no = no
deviceinfo-refresh = Refresh

# --- day-piece-activity ---
nav-activity = Activity
activity-caption = An indeterminate spinner shows work of unknown length.
activity-animating = Animating
activity-on = Spinning
activity-off = Stopped
activity-large-label = Large

# --- day-piece-searchfield ---
nav-search = Search
search-caption = Type to filter the list; the result label shows the first match.
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
