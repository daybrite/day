// Prints the window number of the LARGEST on-screen window owned by a pid (for screencapture -l).
import CoreGraphics
import Foundation
guard CommandLine.arguments.count >= 2, let pid = Int32(CommandLine.arguments[1]) else {
    FileHandle.standardError.write(Data("usage: winid <pid>\n".utf8)); exit(2)
}
let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
let windows = (CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]]) ?? []
var best: (id: Int, area: CGFloat)?
for window in windows {
    guard let owner = window[kCGWindowOwnerPID as String] as? pid_t, owner == pid,
          let number = window[kCGWindowNumber as String] as? Int,
          let bounds = window[kCGWindowBounds as String] as? [String: CGFloat],
          let width = bounds["Width"], let height = bounds["Height"] else { continue }
    let area = width * height
    if best == nil || area > best!.area { best = (number, area) }
}
if let best { print(best.id); exit(0) }
exit(1)
