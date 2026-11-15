import AppKit
import Foundation

/*
 * iconmaker <input|-> <output.png> [accent-hex] [mode] [monogram]
 *
 * Clean app icons in the style Chrome uses for installed PWAs on macOS:
 * the site's own icon inside the macOS icon shape (824px continuous-corner
 * squircle on a 1024px canvas, standard drop shadow), with no gloss.
 *
 * mode:
 *   auto      opaque square artwork fills the shape; artwork with
 *             transparency sits on a white tile (Chrome's behaviour)
 *   fill      artwork fills the shape (maskable / full-bleed icons)
 *   logo      artwork on a white tile, trimmed and centred
 *   monogram  flat accent tile with one letter (no usable icon)
 */

let canvasSize: CGFloat = 1024
let shapeSize: CGFloat = 824          // macOS icon grid (Big Sur and later)
let shape = NSRect(
    x: (canvasSize - shapeSize) / 2,
    y: (canvasSize - shapeSize) / 2 + 10, // optical lift for the shadow
    width: shapeSize,
    height: shapeSize
)

func color(_ hex: String) -> NSColor {
    var text = hex.trimmingCharacters(in: .whitespacesAndNewlines)
    if text.hasPrefix("#") { text.removeFirst() }
    guard text.count == 6, let value = Int(text, radix: 16) else {
        return NSColor(srgbRed: 0.36, green: 0.55, blue: 1.0, alpha: 1)
    }
    return NSColor(
        srgbRed: CGFloat((value >> 16) & 0xff) / 255.0,
        green: CGFloat((value >> 8) & 0xff) / 255.0,
        blue: CGFloat(value & 0xff) / 255.0,
        alpha: 1
    )
}

// Continuous-corner squircle (superellipse, n = 5), close to Apple's icon mask.
func squircle(_ rect: NSRect) -> NSBezierPath {
    let path = NSBezierPath()
    let n: CGFloat = 5
    let a = rect.width / 2
    let b = rect.height / 2
    let steps = 720
    for i in 0...steps {
        let t = CGFloat(i) / CGFloat(steps) * 2 * .pi
        let c = cos(t)
        let s = sin(t)
        let x = pow(abs(c), 2 / n) * a * (c < 0 ? -1 : 1)
        let y = pow(abs(s), 2 / n) * b * (s < 0 ? -1 : 1)
        let point = NSPoint(x: rect.midX + x, y: rect.midY + y)
        if i == 0 { path.move(to: point) } else { path.line(to: point) }
    }
    path.close()
    return path
}

func bitmapOf(_ image: NSImage) -> NSBitmapImageRep? {
    if let rep = image.representations
        .compactMap({ $0 as? NSBitmapImageRep })
        .max(by: { $0.pixelsWide < $1.pixelsWide }) {
        return rep
    }
    guard let tiff = image.tiffRepresentation else { return nil }
    return NSBitmapImageRep(data: tiff)
}

// True when the artwork reaches all four corners opaquely (a full-bleed tile).
func isOpaqueSquare(_ rep: NSBitmapImageRep) -> Bool {
    let w = rep.pixelsWide
    let h = rep.pixelsHigh
    guard w > 0, h > 0, abs(w - h) <= max(2, w / 20) else { return false }
    let inset = max(1, w / 16)
    let points = [(inset, inset), (w - 1 - inset, inset), (inset, h - 1 - inset), (w - 1 - inset, h - 1 - inset),
                  (w / 2, inset), (w / 2, h - 1 - inset), (inset, h / 2), (w - 1 - inset, h / 2)]
    for (x, y) in points {
        guard let c = rep.colorAt(x: x, y: y), c.alphaComponent > 0.9 else { return false }
    }
    return true
}

// Bounding box of visible pixels, in the image's point space.
func contentBounds(_ rep: NSBitmapImageRep, _ size: NSSize) -> NSRect {
    let w = rep.pixelsWide
    let h = rep.pixelsHigh
    var minX = w, minY = h, maxX = -1, maxY = -1
    let step = max(1, min(w, h) / 256)
    var y = 0
    while y < h {
        var x = 0
        while x < w {
            if let c = rep.colorAt(x: x, y: y), c.alphaComponent > 0.08 {
                minX = min(minX, x); maxX = max(maxX, x)
                minY = min(minY, y); maxY = max(maxY, y)
            }
            x += step
        }
        y += step
    }
    guard maxX >= minX, maxY >= minY else { return NSRect(origin: .zero, size: size) }
    let sx = size.width / CGFloat(w)
    let sy = size.height / CGFloat(h)
    // Bitmap rows run top-down; image space runs bottom-up.
    return NSRect(
        x: CGFloat(minX) * sx,
        y: CGFloat(h - 1 - maxY) * sy,
        width: CGFloat(maxX - minX + step) * sx,
        height: CGFloat(maxY - minY + step) * sy
    )
}

let argv = CommandLine.arguments
guard argv.count >= 3 else {
    FileHandle.standardError.write(Data("usage: iconmaker <input|-> <output> [accent] [auto|fill|logo|monogram] [letter]\n".utf8))
    exit(2)
}

let inputPath = argv[1]
let outputPath = argv[2]
let accent = color(argv.count > 3 ? argv[3] : "#5b8cff")
let requested = argv.count > 4 ? argv[4] : "auto"
let monogram = argv.count > 5 ? String(argv[5].prefix(1)).uppercased() : "Z"

let image: NSImage? = inputPath == "-" ? nil : NSImage(contentsOfFile: inputPath)
let rep = image.flatMap(bitmapOf)

var mode = requested
if image == nil { mode = "monogram" }
// Older callers passed "full" / "tile".
if mode == "full" { mode = "fill" }
if mode == "tile" { mode = "logo" }
if mode == "auto" { mode = (rep.map(isOpaqueSquare) ?? false) ? "fill" : "logo" }

let bitmap = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: Int(canvasSize),
    pixelsHigh: Int(canvasSize),
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
)!

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
NSGraphicsContext.current?.imageInterpolation = .high

let mask = squircle(shape)

// Standard macOS icon shadow, drawn once under the tile.
NSGraphicsContext.saveGraphicsState()
let shadow = NSShadow()
shadow.shadowColor = NSColor(white: 0, alpha: 0.28)
shadow.shadowBlurRadius = 24
shadow.shadowOffset = NSSize(width: 0, height: -10)
shadow.set()
(mode == "monogram" ? accent : NSColor.white).setFill()
mask.fill()
NSGraphicsContext.restoreGraphicsState()

NSGraphicsContext.saveGraphicsState()
mask.addClip()

switch mode {
case "fill":
    if let image {
        let source = image.size
        let scale = max(shape.width / source.width, shape.height / source.height)
        let size = NSSize(width: source.width * scale, height: source.height * scale)
        image.draw(
            in: NSRect(x: shape.midX - size.width / 2, y: shape.midY - size.height / 2, width: size.width, height: size.height),
            from: .zero, operation: .sourceOver, fraction: 1
        )
    }
case "logo":
    if let image {
        let bounds = rep.map { contentBounds($0, image.size) } ?? NSRect(origin: .zero, size: image.size)
        // Logo fits a 60% box, but never more than 8x its pixel size (keeps tiny favicons crisp).
        let box = shape.width * 0.60
        let pixelScale = rep.map { CGFloat($0.pixelsWide) / max(image.size.width, 1) } ?? 1
        let maxScale = 8 * pixelScale * (canvasSize / 1024)
        let scale = min(box / max(bounds.width, 1), box / max(bounds.height, 1), maxScale)
        let size = NSSize(width: bounds.width * scale, height: bounds.height * scale)
        image.draw(
            in: NSRect(x: shape.midX - size.width / 2, y: shape.midY - size.height / 2, width: size.width, height: size.height),
            from: bounds, operation: .sourceOver, fraction: 1
        )
    }
default: // monogram
    let paragraph = NSMutableParagraphStyle()
    paragraph.alignment = .center
    let text = NSAttributedString(string: monogram, attributes: [
        .font: NSFont.systemFont(ofSize: shape.width * 0.5, weight: .semibold),
        .foregroundColor: NSColor.white,
        .paragraphStyle: paragraph,
    ])
    let height = text.size().height
    text.draw(in: NSRect(x: shape.minX, y: shape.midY - height / 2, width: shape.width, height: height))
}
NSGraphicsContext.restoreGraphicsState()

// Hairline so a white tile still has an edge on a light Dock.
NSColor(white: 0, alpha: 0.07).setStroke()
let edge = squircle(shape.insetBy(dx: 0.75, dy: 0.75))
edge.lineWidth = 1.5
edge.stroke()

NSGraphicsContext.restoreGraphicsState()

guard let data = bitmap.representation(using: .png, properties: [:]) else {
    FileHandle.standardError.write(Data("iconmaker: could not encode PNG\n".utf8))
    exit(1)
}
do {
    try data.write(to: URL(fileURLWithPath: outputPath))
} catch {
    FileHandle.standardError.write(Data("iconmaker: \(error)\n".utf8))
    exit(1)
}
print(mode)
