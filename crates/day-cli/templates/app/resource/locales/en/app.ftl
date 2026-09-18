# {{title}}: UI strings (https://daybrite.dev/docs/localization). Add a locale by dropping a
# sibling folder (e.g. locales/fr/app.ftl) and translating; the generated
# res::locales::install() in src/lib.rs picks up every locale directory by itself.
#
# The appearance and language rows on the Settings page label themselves from Day's own catalog,
# so there are no keys for them here.

app_title = {{title}}

nav_welcome = Welcome
nav_navigate = Navigate
nav_settings = Settings

# The Welcome page. `welcome_body` is rendered as markdown, so the emphasis lives here rather
# than in the layout, so a translation is free to stress a different word.
# Each paragraph is one line: Fluent keeps the line breaks you write, so a value wrapped for the
# editor's margin would be wrapped that way on screen too, mid-sentence.
welcome_title = Welcome to Day
welcome_body =
    This is a starting point for your next app. We’ve included a few everyday features so you have something to try, explore, and make your own.

    Open [**Navigate**](#navigate) to add and edit items, or visit [**Settings**](#settings) to change the appearance. Ready to start building? You’ll find guides at [daybrite.dev](https://daybrite.dev).

# Menus and commands. One string per command, shared by the menu bar, the toolbar, and the row
# context menus, so a command reads the same wherever the user finds it.
menu_file = File
menu_edit = Edit
cmd_add = New Item
cmd_delete = Delete
cmd_done = Done
cmd_show_done = Show Finished

# The item list and its editor.
item_none = Select an item
item_kind_note = Note
item_kind_task = Task
item_kind_idea = Idea

section_basics = Basics
section_details = Details
section_notes = Notes

field_name = Name
field_name_hint = Name this item…
field_count = Count
field_date = Date
field_kind = Kind
field_done = Done
field_rating = Rating
field_color = Color
