// The day Runner (DESIGN.md §17.1): a thin native shell. The Rust staticlib (built by the
// `day xcode-backend build` script phase) exports day_main, which boots the AppKit app.
import AppKit

@_silgen_name("day_main")
func day_main()

day_main()
