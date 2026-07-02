// imgstat.swift — print "<width> <height> <distinct-color-count>" for a PNG (macOS CI has no
// ImageMagick; Linux uses `identify -format '%w %h %k'` instead). Used by
// scripts/ci/validate-screenshots.sh to reject blank/transparent captures (§20).
import Foundation
import CoreGraphics
import ImageIO

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: imgstat.swift <image.png>\n".utf8))
    exit(2)
}
let url = URL(fileURLWithPath: CommandLine.arguments[1])
guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
      let img = CGImageSourceCreateImageAtIndex(src, 0, nil)
else {
    FileHandle.standardError.write(Data("imgstat: cannot decode \(url.path)\n".utf8))
    exit(1)
}
let w = img.width, h = img.height
var pixels = [UInt32](repeating: 0, count: w * h)
guard let ctx = CGContext(
    data: &pixels, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
    space: CGColorSpace(name: CGColorSpace.sRGB)!,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else {
    FileHandle.standardError.write(Data("imgstat: cannot rasterize \(url.path)\n".utf8))
    exit(1)
}
ctx.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
print("\(w) \(h) \(Set(pixels).count)")
