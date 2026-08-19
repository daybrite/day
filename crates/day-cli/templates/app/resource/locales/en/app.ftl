# {{title}} — UI strings (https://daybrite.dev/docs/localization). Add a locale by dropping a
# sibling folder (e.g. locales/fr/app.ftl) and translating — the generated
# res::locales::install() in src/lib.rs picks up every locale directory by itself.
#
# The appearance and language rows on the Settings page label THEMSELVES from Day's own catalog,
# so there are no keys for them here.

app_title = {{title}}

nav_welcome = Welcome
nav_navigate = Navigate
nav_settings = Settings

# The Welcome page. `welcome_body` is rendered as markdown, so the emphasis lives here rather
# than in the layout — a translation is free to stress a different word.
# Each paragraph is ONE line: Fluent keeps the line breaks you write, so a value wrapped for the
# editor's margin would be wrapped that way on screen too, mid-sentence.
welcome_title = Welcome to Day
welcome_body =
    Glad you are here. This little app is yours to take apart — every screen in it is a few lines of Rust, and the widgets you are looking at are your platform's own.

    Open **Navigate** for a list you can reorder, edit, and drill into, then resize the window and watch the navigation find a new shape. When you are ready to build something, the guides are at [daybrite.dev](https://daybrite.dev).

# Menus and commands. One string per command, shared by the menu bar, the toolbar, and the row
# context menus, so a command reads the same wherever the user finds it.
menu_file = File
menu_edit = Edit
cmd_add = New Item
cmd_delete = Delete
cmd_done = Done
cmd_show_done = Show Finished
cmd_sidebar = Toggle Sidebar

# The item list and its editor.
item_untitled = Untitled
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
