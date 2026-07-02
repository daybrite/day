// axpress <pid> <ax-identifier> [count] — find the element by AXIdentifier in the app's AX tree
// and perform AXPress on it `count` times. Exit 0 on success.
import ApplicationServices
import Foundation

func find(_ el: AXUIElement, _ ident: String, depth: Int = 0) -> AXUIElement? {
    if depth > 25 { return nil }
    var idVal: CFTypeRef?
    if AXUIElementCopyAttributeValue(el, "AXIdentifier" as CFString, &idVal) == .success,
       let s = idVal as? String, s == ident { return el }
    var kids: CFTypeRef?
    if AXUIElementCopyAttributeValue(el, kAXChildrenAttribute as CFString, &kids) == .success,
       let arr = kids as? [AXUIElement] {
        for k in arr { if let hit = find(k, ident, depth: depth + 1) { return hit } }
    }
    return nil
}

guard CommandLine.arguments.count >= 3, let pid = Int32(CommandLine.arguments[1]) else {
    FileHandle.standardError.write(Data("usage: axpress <pid> <identifier> [count]\n".utf8)); exit(2)
}
let ident = CommandLine.arguments[2]
let count = CommandLine.arguments.count > 3 ? Int(CommandLine.arguments[3]) ?? 1 : 1
let app = AXUIElementCreateApplication(pid)
var winsRef: CFTypeRef?
var target: AXUIElement?
if AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &winsRef) == .success,
   let arr = winsRef as? [AXUIElement] {
    for w in arr { if let hit = find(w, ident) { target = hit; break } }
}
guard let el = target else {
    FileHandle.standardError.write(Data("not found: \(ident)\n".utf8)); exit(1)
}
for _ in 0..<count {
    let r = AXUIElementPerformAction(el, kAXPressAction as CFString)
    if r != .success { FileHandle.standardError.write(Data("press failed: \(r.rawValue)\n".utf8)); exit(1) }
    usleep(120_000)
}
print("pressed \(ident) x\(count)")
