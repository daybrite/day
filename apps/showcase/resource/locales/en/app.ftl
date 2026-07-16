app_title = Day Showcase
counter_value = { $count ->
    [one] { $count } click
   *[other] { $count } clicks
}
decrement = −
increment = +
name_placeholder = Your name
greeting = Hello, { $name }!
volume_label = Volume
progress_label = Progress
busy_label = Busy
flavor_label = Flavor
history_title = History
history_entry = count became { $value }
nav_controls = Controls
nav_menus = Menus & dialogs
nav_text = Text
nav_battery = Battery
nav_sensors = Sensors
nav_clipboard = Clipboard
nav_network = Network
nav_media = Media
nav_pickers = Pickers
nav_compose = Compose
nav_files = Files
nav_tabs = Tabs
nav_stack = Stack
nav_list = List
nav_webview = Web View
nav_lottie = Lottie
nav_about = About

shapes_kinds = Kinds
gradients_title = Gradients
gradient_angle = Angle
shapes_transform = Transform
shapes_angle = Angle

picker_shared_caption = All three stylings are bound to the same selection signal — change one and the others follow.
picker_selected = Selected
picker_segmented = Segmented
picker_menu = Menu
picker_inline = Inline

compose_caption = Pure-composition pieces — no native code, no cargo features, every backend for free.
compose_rating_label = Star rating
compose_rating_count = Stars selected:
compose_rating_placeholder = 1–5
compose_card_title = Reusable surface
compose_card_body = Padding + background + rounded corners, applied as a Modifier.
compose_plain_btn = Plain
compose_styled_btn = Filled
compose_env_value = Tinted by the provided accent
list_add = Add 100
list_caption = { $count } rows — only the visible cells are built

webview_url_hint = Enter a URL
webview_go = Go
webview_back = Back
webview_forward = Forward
webview_stop = Stop
webview_reload = Reload

lottie_caption = A native Lottie animation, bundled as JSON (lottie-ios / lottie-android)
lottie_speed = Speed
stack_root_body = A genuine push/pop stack. Its path is an app-owned signal.
stack_push = Push a detail
stack_detail_title = Level { $depth }
stack_detail_body = Pushed onto the path. The native back button writes the pop back.
stack_item_title = Item { $id }
stack_link_42 = Open item-42 with a hint (absolute route)
stack_param_hint = Opened with hint: {$hint}
tab_one = Overview
tab_two = Details
tab_three = Settings
tab_one_body = The overview tab. Each tab keeps its own state.
tab_two_body = The details tab, selected by its route key.
tab_three_body = The settings tab. Deep links and dayscript select tabs by key.
about_text = A native cross-platform app built with day.
modal_alert = Show alert
modal_confirm = Confirm
modal_delete = Delete…
modal_sheet = Pick flavor
modal_prompt = Enter name
alert_title = Notice
alert_body = Your changes have been saved.
ok = OK
confirm_title = Quit?
confirm_body = Are you sure you want to quit?
delete_title = Delete item?
delete_body = This cannot be undone.
delete = Delete
flavor_title = Choose a flavor
cancel = Cancel
vanilla = vanilla
pistachio = pistachio

# Files playground (docs/files.md)
files_caption = Native open/save file pickers. Open reads a text file into the editor; Save writes it back out.
files_placeholder = Type something to save…
files_open = Open File…
files_save = Save File…
files_opened = Opened { $name }

# Battery playground (docs/battery.md)
battery_refresh = Read Device Battery
battery_level = Level
battery_charging = Charging
battery_reading = Battery: { $percent } · { $state }
battery_reading_none = Battery: no battery API on this platform

# Sensors playground (docs/sensors.md)
sensors_refresh = Read Sensors
sensor_accelerometer = Accelerometer
sensor_gyroscope = Gyroscope
sensor_magnetometer = Magnetometer
sensor_reading = x { $x } · y { $y } · z { $z } { $unit }
sensor_waiting = waiting for first sample…
sensor_unavailable = unavailable on this device

# Clipboard playground (docs/clipboard.md)
clipboard_caption = The day-part-clipboard part reads and writes the system clipboard natively.
clipboard_placeholder = Type something to copy
clipboard_copy = Copy
clipboard_paste = Paste
clipboard_idle = Clipboard untouched
clipboard_copied = Copied to the system clipboard
clipboard_copy_failed = Copy failed (no clipboard API here)
clipboard_pasted = Pasted from the system clipboard
clipboard_empty = Clipboard is empty (or unreadable in the background)

# Network playground (docs/network.md)
network_refresh = Read Network
network_reading_online = Online · { $kind } · metered: { $expensive }
network_reading_offline = Offline
network_reading_none = No connectivity API on this platform

# Media playground (docs/media.md)
media_play = Play
media_pause = Pause
media_load = Load

# Text playground (typography)
text_caption = Semantic styles map to the platform's native text styles and accessibility text scaling.
text_styles_header = Styles
text_weights_header = Weights
text_styling_header = Bold & italic
text_colors_header = Color
text_custom_header = Custom sizes
text_custom_note = Font.System(pt) — still scaled by the accessibility text size (Dynamic Type / font scale).
text_fonts_header = Bundled fonts
text_fonts_note = Font.Custom("Family", pt) — files from the app's resource/fonts/ directory, bundled by day build and resolved by family name on every platform.

# Menus playground
menus_caption = The transient native surfaces: the menu bar, per-piece context menus, and imperative dialogs.
menus_last = Last action
menus_lifecycle = Lifecycle
menus_target = Right-click here (long-press on mobile) for a context menu
menus_shortcut_hint = Keyboard shortcuts (⌘/Ctrl + key) are shown in the menu bar and work while the app is focused — e.g. New (N), Save (S), Reload (R), Save As (⇧S).

# --- day-part-haptics ---
nav_haptics = Haptics
haptics_supported_yes = Haptic engine available on this platform
haptics_supported_no = No haptic engine on this platform (buttons are silent)
haptics_light = Light
haptics_medium = Medium
haptics_heavy = Heavy
haptics_success = Success
haptics_warning = Warning
haptics_error = Error
haptics_selection = Selection
haptics_last = Last played
haptics_none = Nothing played yet
haptics_last_played = Played: { $style }

# --- day-part-prefs ---
nav_prefs = Preferences
prefs_caption = Persist a string across launches with day-part-prefs.
prefs_placeholder = Value to remember
prefs_save = Save
prefs_load = Load
prefs_clear = Clear
prefs_idle = Type a value, then Save.
prefs_empty = (nothing stored)
prefs_saved = Saved.
prefs_save_failed = Save failed.
prefs_loaded = Loaded from storage.
prefs_missing = Nothing stored yet.
prefs_cleared = Cleared.
prefs_value_label = Stored value:

# --- bundled resources (§18.3) ---
nav_resources = Resources
resources_caption = An image loaded by name from a bundled resource, plus random-access reads of embedded data.
resources_numbers = numbers.bin: { $len } bytes, byte[100] = { $byte }
resources_greeting = greeting.txt: { $text }

# --- day-part-deviceinfo ---
nav_deviceinfo = Device Info
deviceinfo_model = Model: {$value}
deviceinfo_system = System: {$name} {$version}
deviceinfo_simulator = Simulator: {$value}
deviceinfo_yes = yes
deviceinfo_no = no
deviceinfo_refresh = Refresh

# --- day-piece-activity ---
activity_animating = Animating
activity_on = Spinning
activity_off = Stopped

# --- day-piece-searchfield ---
nav_search = Search
search_placeholder = Search fruit…
search_clear = Clear

# --- day-piece-map ---
nav_map = Map
map_caption = A native MKMapView — Apple platforms only. Tap a preset to recenter the map live.
map_sf = San Francisco
map_nyc = New York

# — tweaks page (docs/tweaks.md) —
nav_tweaks = Tweaks
tweaks_intro = Packaged tweaks configure the native widget behind a built-in piece, per toolkit. On toolkits a tweak doesn't cover, it is a no-op — the pieces below simply look stock.
tweaks_stock = Stock
tweaks_tweaked = Tweaked
tweaks_bezel_title = Button bezel
tweaks_bezel_caption = day-tweak-button-bezel — AppKit only: NSBezelStyle constants on the real NSButton.
tweaks_selectable_title = Selectable label
tweaks_selectable_caption = day-tweak-label-selectable — AppKit, GTK, Android: the platform's own text selection on a stock label.
tweaks_selectable_text = This label's text can be selected and copied — try it.
tweaks_ticks_title = Slider tick marks
tweaks_ticks_caption = day-tweak-slider-tickmarks — AppKit, GTK, Android, Qt, WinUI, ArkUI: native ticks, snapping where the platform supports it. The tweaked slider snaps; the stock one glides.
tweaks_ref_title = NativeRef liveness
tweaks_ref_caption = A NativeRef reaches the tweaked slider after mount; unmount it and the ref clears instead of dangling.
tweaks_ref_live = ref: live
tweaks_ref_cleared = ref: cleared

# — merged section pages (design overhaul) —
nav_canvas = Canvas & shapes
nav_system = Device & sensors
nav_services = Platform services
controls_caption = Two-way bindings: every control is a projection of an app-owned signal.
controls_basics = Basics
controls_feedback = Feedback
canvas_caption = Shapes, transforms, gestures, and composition-tier widgets — all drawn through the canvas.
canvas_gauge = Canvas gauge
shapes_interact_hint = Drag the slider to rotate, tap the circle to recolor, drag the purple square to move it.
system_caption = The headless device-state parts: battery, connectivity, motion sensors, and device identity.
services_caption = The headless "do something with the OS" parts: clipboard, preferences, haptics, and file pickers.
subscribe_label = Subscribe

# — data strings localized for the walkthrough locales (option lists, specimen rows) —
chocolate = chocolate
size_small = Small
size_medium = Medium
size_large = Large
fruit_apple = Apple
fruit_banana = Banana
fruit_cherry = Cherry
fruit_date = Date
fruit_elderberry = Elderberry
list_row = Row { $n }
text_style_large_title = Large Title
text_style_title = Title
text_style_title2 = Title 2
text_style_title3 = Title 3
text_style_headline = Headline
text_style_subheadline = Subheadline
text_style_body = Body
text_style_callout = Callout
text_style_footnote = Footnote
text_style_caption = Caption
text_style_caption2 = Caption 2
text_weight_ultralight = Ultra Light
text_weight_light = Light
text_weight_regular = Regular
text_weight_medium = Medium
text_weight_semibold = Semibold
text_weight_bold = Bold
text_weight_heavy = Heavy
text_weight_black = Black
text_bold = Bold text
text_italic = Italic text
text_bolditalic = Bold italic
text_emphasis_label = Emphasis
color_red = Red
color_green = Green
color_blue = Blue
color_orange = Orange

# Menus & dialogs (merged page)
menus_appmenu_section = App menu
menus_context_section = Context menu
menus_dialogs_section = Dialogs
modal_result_label = Result

# Media page
media_caption = A native media player — the platform's own view, transport driven by triggers.
media_player_section = Video

# Resources page sections
resources_image_section = Bundled image
resources_modes_note = One image, three content modes — Fit preserves aspect, Fill crops, Stretch distorts.
image_mode_fit = Fit
image_mode_fill = Fill
image_mode_stretch = Stretch
resources_data_section = Data assets

# About page
about_caption = What this app is, and the platform it landed on.
about_app_section = This app
about_version = Version
about_toolkit = Toolkit
about_battery = Battery
history_hint = Tap + or − above and each change lands here.

# Focus page (docs/focus.md)
nav_focus = Focus
focus_caption = Focus is a two-way binding: native changes write the signal, and writing the signal moves focus.
focus_group_section = One signal, one form
focus_group_caption = Three fields bound to one optional enum signal. Click or Tab between them and the readout follows; Return hops to the next field.
focus_name_label = Name
focus_email_label = Email
focus_city_label = City
focus_current_label = Focused
focus_next = Focus next
focus_clear = Clear focus
focus_bool_section = One control, one Bool
focus_bool_caption = The same field bound to a Bool signal — the buttons write it; clicking in and out of the field writes it back.
focus_bool_placeholder = Focus lands here
focus_focus_btn = Focus
focus_blur_btn = Blur
focus_state_label = State
focus_state_on = focused
focus_state_off = blurred
focus_probe_section = Beyond text fields
focus_probe_caption = Desktop toolkits focus buttons, toggles, and sliders too; touch platforms mostly reserve focus for text input.
focus_probe_toggle = Toggle
focus_probe_slider = Slider
focus_probe_button = Button
