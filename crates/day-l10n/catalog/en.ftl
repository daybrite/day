# Day core UI catalog — standard strings the framework itself needs (dialog buttons, standard menu
# commands). Keys are namespaced `day-*` so an app's own catalog never clashes; an app CAN override
# any of these by defining the same key in its own `.ftl`. English is the ultimate fallback.

# Dialog buttons (docs/dialogs.md)
day-ok = OK
day-cancel = Cancel
day-yes = Yes
day-no = No
day-done = Done
day-save = Save
day-open = Open
day-close = Close
day-delete = Delete

# Standard menu commands (docs/menus.md — MenuRole)
day-cut = Cut
day-copy = Copy
day-paste = Paste
day-select-all = Select All
day-undo = Undo
day-redo = Redo
day-about = About
day-quit = Quit
day-preferences = Preferences
day-minimize = Minimize
day-fullscreen = Enter Full Screen

# Standard menu titles + app-name commands (used by the AppKit default/standard app menu)
day-edit = Edit
day-about-app = About {$app}
day-quit-app = Quit {$app}
