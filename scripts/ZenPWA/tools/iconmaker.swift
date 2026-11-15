import AppKit
import Foundation

/*
 * iconmaker <input.png|jpg|-> <output.png> [accent-hex] [mode] [monogram]
 *
 * mode:
 *   full      draw the artwork edge to edge (use for >=256px icons)
 *   tile      accent gradient tile with the artwork centred (small favicons)
 *   monogram  accent gradient tile with a single letter
 *
 * Every icon keeps the macOS margin (100px on a 1024px canvas) and a
 * squircle-ish corner radius, so it sits beside native apps naturally.
 */

let canvasSize: CGFloat = 1024
let margin: CGFloat = 100
let tile = NSRect(x: margin, y: margin, width: canvasSize - margin * 2, height: canvasSize - margin * 2)
let radius: CGFloat = 186

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

func mix(_ a: NSColor, _ b: NSColor, _ amount: CGFloat) -> NSColor {
    let x = a.usingColorSpace(.sRGB) ?? a
    let y = b.usingColorSpace(.sRGB) ?? b
    let t = max(0, min(1, amount))
    return NSColor(
        srgbRed: x.redComponent + (y.redComponent - x.redComponent) * t,
        green: x.greenComponent + (y.greenComponent - x.greenComponent) * t,
        blue: x.blueComponent + (y.blueComponent - x.blueComponent) * t,
        alpha: 1
    )
}

func roundedPath(_ rect: NSRect, _ corner: CGFloat) -> NSBezierPath {
    NSBezierPath(roundedRect: rect, xRadius: corner, yRadius: corner)
}

let argv = CommandLine.arguments
guard argv.count >= 3 else {
    FileHandle.standardError.write(Data("usage: iconmaker <input> <output> [accent] [mode] [monogram]\n".utf8))
    exit(2)
}

let inputPath = argv[1]
let outputPath = argv[2]
let accent = color(argv.count > 3 ? argv[3] : "#5b8cff")
let mode = argv.count > 4 ? argv[4] : "tile"
let monogram = argv.count > 5 ? String(argv[5].prefix(1)).uppercased() : "Z"

let image: NSImage? = inputPath == "-" ? nil : NSImage(contentsOfFile: inputPath)
let effectiveMode = (mode == "full" && image == nil) ? "monogram" : mode

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

let clip = roundedPath(tile, radius)

if effectiveMode == "full", let image {
    clip.addClip()
    let source = image.size
    let scale = max(tile.width / source.width, tile.height / source.height)
    let size = NSSize(width: source.width * scale, height: source.height * scale)
    let rect = NSRect(
        x: tile.midX - size.width / 2,
        y: tile.midY - size.height / 2,
        width: size.width,
        height: size.height
    )
    image.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1)
    // Gentle inner lip keeps flat artwork from looking pasted on.
    NSColor(white: 1, alpha: 0.10).setStroke()
    let lip = roundedPath(tile.insetBy(dx: 1, dy: 1), radius - 1)
    lip.lineWidth = 2
    lip.stroke()
} else {
    let top = mix(accent, .white, 0.20)
    let bottom = mix(accent, .black, 0.34)
    let gradient = NSGradient(starting: top, ending: bottom)!
    clip.addClip()
    gradient.draw(in: tile, angle: -90)

    // Soft light from the top-left, like macOS app icons.
    let glow = NSGradient(colors: [
        NSColor(white: 1, alpha: 0.22),
        NSColor(white: 1, alpha: 0.0),
    ])!
    glow.draw(in: NSRect(x: tile.minX, y: tile.midY, width: tile.width, height: tile.height / 2), angle: -90)

    if effectiveMode != "monogram", let image {
        let target = tile.width * 0.60
        let source = image.size
        let scale = min(target / max(source.width, 1), target / max(source.height, 1))
        let size = NSSize(width: source.width * scale, height: source.height * scale)
        let rect = NSRect(
            x: tile.midX - size.width / 2,
            y: tile.midY - size.height / 2,
            width: size.width,
            height: size.height
        )
        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow()
        shadow.shadowColor = NSColor(white: 0, alpha: 0.34)
        shadow.shadowBlurRadius = 34
        shadow.shadowOffset = NSSize(width: 0, height: -10)
        shadow.set()
        let artworkClip = roundedPath(rect.insetBy(dx: -size.width * 0.055, dy: -size.height * 0.055), size.width * 0.16)
        artworkClip.addClip()
        image.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()
    } else {
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        let font = NSFont.systemFont(ofSize: tile.width * 0.46, weight: .semibold)
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font,
            .foregroundColor: NSColor(white: 1, alpha: 0.96),
            .paragraphStyle: paragraph,
        ]
        let text = NSAttributedString(string: monogram, attributes: attributes)
        let height = text.size().height
        text.draw(in: NSRect(
            x: tile.minX,
            y: tile.midY - height / 2,
            width: tile.width,
            height: height
        ))
    }

    NSColor(white: 1, alpha: 0.12).setStroke()
    let lip = roundedPath(tile.insetBy(dx: 1, dy: 1), radius - 1)
    lip.lineWidth = 2
    lip.stroke()
}

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
