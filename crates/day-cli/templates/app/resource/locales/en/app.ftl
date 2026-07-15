# {{title}} — UI strings (https://daybrite.dev/docs/localization). Add a locale by dropping a
# sibling folder (e.g. locales/fr/app.ftl) and registering it in src/lib.rs.

app_title = {{title}}

nav_home = Home
nav_controls = Controls
nav_canvas = Canvas
nav_items = Items

home_welcome = Welcome to {{title}}
home_blurb = This app is a Day starter: a typed-route sidebar over four sample panels. Everything below is an app-owned signal projected into native widgets.

controls_title = Controls
controls_field_placeholder = Type something…
controls_echo = You typed:

canvas_title = Canvas
canvas_blurb = A reactive display list: the draw closure reads the slider's signal, so dragging redraws the dial natively on every platform.

items_title = Items
items_blurb = A push/pop stack bound to a Signal of typed routes. The native back button (and back gesture) writes the pop back into the path.
item_open = Open item { $id }
item_link = Open item 3 by absolute route
item_title = Item { $id }
item_body = Pushed onto the path as a typed value — no string splitting.
item_via = Opened via: { $via }
item_home = Go home (typed relative route)
