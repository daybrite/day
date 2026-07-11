# {{title}} — UI strings (https://daybrite.dev/docs/localization). Add a locale by dropping a
# sibling folder (e.g. locales/fr/app.ftl) and registering it in src/lib.rs.

app-title = {{title}}

nav-home = Home
nav-controls = Controls
nav-canvas = Canvas
nav-items = Items

home-welcome = Welcome to {{title}}
home-blurb = This app is a Day starter: a typed-route sidebar over four sample panels. Everything below is an app-owned signal projected into native widgets.

controls-title = Controls
controls-field-placeholder = Type something…
controls-echo = You typed:

canvas-title = Canvas
canvas-blurb = A reactive display list: the draw closure reads the slider's signal, so dragging redraws the dial natively on every platform.

items-title = Items
items-blurb = A push/pop stack bound to a Signal of typed routes. The native back button (and back gesture) writes the pop back into the path.
item-open = Open item { $id }
item-link = Open item 3 by absolute route
item-title = Item { $id }
item-body = Pushed onto the path as a typed value — no string splitting.
item-via = Opened via: { $via }
item-home = Go home (typed relative route)
