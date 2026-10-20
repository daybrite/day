---
title: "Fluent catalog for documentation examples"
description: "Message definitions and setup for the generated string accessors used in Day's UI examples."
---

# Fluent catalog for documentation examples

The UI snippets use app-generated `res::str` functions. These names are **example messages,
not built-in Day strings**. Add the entries you need to
`resource/locales/en-US/app.ftl` (or your app's default locale), retaining your existing
messages. Define the same keys and arguments in the other locales you ship.

The scaffold's `build.rs` calls `day_build::prebuild_project()` and its library declares
`day::resources!();`. In a child module, import `crate::res`; a separate binary imports
`your_app::res` from its library. Start the app with `res::window_options()` so the catalog
and saved language are available before the UI is built.
See [Localization](https://daybrite.dev/docs/localization) for the complete setup.

> [!TIP]
> **Localize the message, not its fragments.** Literal text is accepted by Day's text APIs,
> but app-owned labels, hints, errors, and units should come from the Fluent catalog.
> Pass `Signal<i64>`, `Signal<f64>`, or `Signal<String>` directly to an accessor.
> For a memo, a projected field, or another numeric type, read and convert inside a reactive
> label closure: `label(move || res::str::rating_value(stars.get() as i64).format())`.
> Multi-argument messages use generated named-field structs.

`id!(save_button)` declares a stable `ElementId` independently of the catalog. It names a
control for automation, while `res::str::save()` supplies its translated title.
Recycled rows use `.id_of(move || id!(item_row, slot.key()))`.
See [element IDs](element-ids.md) and [accessibility](accessibility.md).

File paths, route keys, protocol strings, and user-authored content are data, not catalog
entries. The fixed sample amounts and weather readings below are illustrative fixtures.
Bundled images and files in other examples require the corresponding resources in your
project; reference them through `res::images`, `res::vectors`, `res::fonts`, and `res::assets`.

## Example messages

Use this as a lookup for the snippets, not as a catalog to copy wholesale into a shipping app.
Rename messages to suit your screen and give translators context. A guide with its own FTL
block uses that block's definitions.

```ftl
a_quiet_caption = A quiet caption
about = About
about_app = About this app
account = Account
add = Add
add_one = Add one
api_example_org = api.example.org
app_title = Example app
archive = Archive
battery_percent = Battery: { $percent }%
battery_unknown = Battery status unavailable
bold = Bold
bottom = Bottom
brand = Brand
cancel = Cancel
caption = Caption
canvas_value = { $value }
chapter = Chapter
chapter_one = Chapter One
check = Check
city = City
click_count = { $count ->
    [one] One click
   *[other] { $count } clicks
    }
close = Close
connection_closed = Connection closed ({ $code }): { $reason }
counter = Counter
counter_value = Count: { $count }
cpu_usage = CPU usage
custom_size_sample = 18 pt
dark = Dark
decrement = Decrease count
delete = Delete
delete_body = This cannot be undone.
delete_item = Delete item
delete_item_hint = Permanently remove the selected item
delete_title = Delete this item?
discard_changes = Discard changes?
document_content = Document content
drafts = Drafts
drag = Drag
duplicate = Duplicate
edit = Edit
edition_name = { $edition } edition
email = Email
enable_reminders = Enable reminders
enabled = Enabled
everything = Everything
fetch = Fetch
file = File
font_sample = Aa Bb
forecast_rain = Rain later
first_label = a
flash_the_save_button = Flash the save button
good_morning = Good morning
greeting = Hello, { $name }!
grow = Grow
hello = Hello
hello_from_my_first_edit = Hello from my first edit
hello_native_world = Hello, native world
hi = Hi
hint = Changes are saved automatically.
home = Home
how_was_it = How was it?
http_status = HTTP status: { $status }
http_summary = HTTP { $status } · { $bytes } bytes
inbox = Inbox
increment = Increase count
item_100 = Item 100
large = Large
last_updated = Last updated
legalese = legalese
level = Level
library = Library
light = Light
lisbon = Lisbon
loading = Loading…
living_room = Living room
live_quote_title = { $symbol } — live
log_in = Log in
medium = Medium
menu_view = View
mon = Mon
more_actions = More actions
move_to = Move to
move_to_group = Move to group
my_app = My App
nairobi = Nairobi
name = Name
nav_controls = Controls
nav_home = Home
nav_library = Library
nav_settings = Settings
navigation_links = Open [**Navigate**](#navigate) or [Settings](#settings).
new = New
new_item = New Item
new_note = New Note
new_york = New York
next_unread = Next unread
nothing_selected = Nothing selected
notifications = Notifications
online = Online
open = Open…
open_42 = Open 42
open_album = Open album
operation_failed = Could not complete the operation: { $detail }
osaka = Osaka
overwrite = Overwrite?
password = Password
pause = Pause
pause_timer = Pause timer
percentage = { NUMBER($percent, maximumFractionDigits: 0) }%
pick = Pick
pick_a_tint = Pick a tint
pistachio = Pistachio
plan = Plan
play = Play
precipitation = { $percent }%
press = Press
price = Price
pro = Pro
profile = Profile
q3_chart_summary = Sales increased each month from July through September.
qty = Qty
quantity = Quantity
quit = Quit
rating_star = Rate { $stars } out of { $maximum } stars
rating_value = { $stars } / 5
read_me = Read me
received_bytes = Received { $bytes } bytes
record = Record
rectangle = Rectangle
refresh = Refresh
release_note = **New:** Read saved articles offline.
rename = Rename
right_click_me = Right-click me
sample_high = 15°
sample_high_tuesday = 22°
sample_label = x
sample_low = 9°
sample_low_tuesday = 12°
sample_order = Order #A1B2-C3D4
sample_price = { NUMBER($amount, minimumFractionDigits: 2, maximumFractionDigits: 2) }
sample_probability = 40%
sample_total = $1,240.00
san_francisco = San Francisco
save = Save
save_as = Save As…
save_hint = Save your changes
save_instructions = Save it as **notes.md** before you *quit*.
saved = Saved
screenshot = Screenshot
search_fruit = Search fruit…
search_sections = Search sections
second_label = b
send = Send
settings = Settings
settings_title = Settings
show_done = Show completed items
show_password = Show Password
size = Size
small = Small
snooze = Snooze
sound = Sound
stack = Stack
stand_up = Time to stand up
star = Star
standard_edition = Standard edition
start_timer = Start timer
start_typing = Start typing…
stop = Stop
stroke_width = Stroke width
subscribe = Subscribe
system = System
tap_me = Tap me
tell_me_your_name = Tell me your name
temperature = { NUMBER($degrees, maximumFractionDigits: 1) }°
temperature_celsius = { $degrees } °C
terms_blurb = Read the [terms of use](https://example.org/terms).
text_files = Text files
timer_done = Timer done
timer_done_body = Your five-minute timer has finished.
title = Title
total = Total
total_eur = { NUMBER($amount, minimumFractionDigits: 2, maximumFractionDigits: 2) } €
tue = Tue
type_or_pick_a_flavor = Type or pick a flavor
unstar = Unstar
untitled = Untitled
usd = USD
vanilla = Vanilla
view = View
view_both = View both
view_day = View day
view_grid = View grid
volume = Volume
volume_level = Volume level
wake_up = Wake up
weather_title = Weather
welcome_aboard = Welcome aboard
whispered = Whispered
write_a_message = Write a message…
your_name = Your name
about_credits = Created by the community.
about_tagline = Tools for sharing your work.
feeds = Feeds
nav_welcome = Welcome
not_on_this_platform = This view is unavailable on this platform.
pref_snap = Snap to grid
search = Search
settings_editing = Editing
settings_host = Server address
settings_unit = Temperature unit
settings_weather = Weather
smart_feeds = Smart feeds
today = Today
unit_c = Celsius
unit_f = Fahrenheit
unread_feed = Unread
```
