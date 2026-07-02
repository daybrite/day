// axdump <pid> — print the AX tree (role, title/id) to spot identifier plumbing.
import ApplicationServices
import Foundation
func attr(_ el: AXUIElement, _ name: String) -> String {
    var v: CFTypeRef?
    guard AXUIElementCopyAttributeValue(el, name as CFString, &v) == .success else { return "" }
    if let s = v as? String { return s }
    return ""
}
func dump(_ el: AXUIElement, _ depth: Int) {
    if depth > 12 { return }
    let role = attr(el, kAXRoleAttribute as String)
    let id = attr(el, "AXIdentifier")
    let title = attr(el, kAXTitleAttribute as String)
    let val = attr(el, kAXValueAttribute as String)
    print(String(repeating: "  ", count: depth) + "\(role) id=\(id) title=\(title) value=\(val)")
    var kids: CFTypeRef?
    if AXUIElementCopyAttributeValue(el, kAXChildrenAttribute as CFString, &kids) == .success,
       let arr = kids as? [AXUIElement] {
        for k in arr { dump(k, depth + 1) }
    }
}
guard CommandLine.arguments.count >= 2, let pid = Int32(CommandLine.arguments[1]) else { exit(2) }
let app = AXUIElementCreateApplication(pid)
var wins: CFTypeRef?
if AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &wins) == .success,
   let arr = wins as? [AXUIElement] {
    for w in arr { dump(w, 0) }
} else { print("no AXWindows") }
