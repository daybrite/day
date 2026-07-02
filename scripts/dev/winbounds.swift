// Prints "x y w h" of the largest on-screen window owned by a pid.
import CoreGraphics
import Foundation
guard CommandLine.arguments.count >= 2, let pid = Int32(CommandLine.arguments[1]) else { exit(2) }
let windows = (CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]]) ?? []
var best: ([String: CGFloat], CGFloat)?
for w in windows {
    guard let owner = w[kCGWindowOwnerPID as String] as? pid_t, owner == pid,
          let b = w[kCGWindowBounds as String] as? [String: CGFloat],
          let width = b["Width"], let height = b["Height"] else { continue }
    if best == nil || width * height > best!.1 { best = (b, width * height) }
}
if let (b, _) = best { print("\(Int(b["X"]!)) \(Int(b["Y"]!)) \(Int(b["Width"]!)) \(Int(b["Height"]!))"); exit(0) }
exit(1)
