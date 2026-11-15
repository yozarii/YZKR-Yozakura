import AppKit
import Foundation
import ScreenCaptureKit

/*
 * ZenSkin — reskins Safari web apps (Safari → File → Add to Dock) with the
 * Zen compact look, without touching them.
 *
 * Safari keeps doing the hard part (FairPlay DRM, logins, updates). ZenSkin
 * only floats a borderless, click-through overlay exactly over each web-app
 * window and paints what we want on top:
 *   - the Zen frame (space colour, site hue) over the window edges,
 *   - a band over Safari's title bar that fades away while the pointer is at
 *     the top edge, revealing the real title bar and traffic lights,
 *   - nothing at all in fullscreen (so video is untouched).
 * All clicks fall through to Safari, so dragging, buttons and the page work
 * exactly as before.
 *
 * Config: ~/.config/zen-pwa/skin.json (written by ZenPWA/tools/skin-config.js).
 * Targets: every app whose bundle id starts with com.apple.Safari.WebApp
 *          (override for testing: ZENSKIN_TARGET=<bundle id prefix>).
 */

// MARK: - Config

struct SkinConfig {
    var separation: CGFloat = 8
    var innerRadius: CGFloat = 9.1
    var titleBar: CGFloat = 28       // Safari web-app title bar (no nav controls)
    var windowRadius: CGFloat = 16
    var lensLift: CGFloat = 6        // lens top edge rises this far into the hidden title bar
    var lensRadius: CGFloat = 11     // outer lens corner = Zen's window radius (zen.theme.border-radius)
    var cardRadius: CGFloat? = nil   // page corner; default lensRadius minus most of the gap
    var hideDelay: TimeInterval = 1.0
    var defaultFrame = NSColor(srgbRed: 0.44, green: 0.36, blue: 0.41, alpha: 1)
    var frames: [String: NSColor] = [:]   // by app name and by bundle id
    var titleBars: [String: CGFloat] = [:] // per-app title-bar height override
    var stripColors: [String: NSColor] = [:] // fallback title-bar colour (no screen access)

    static let path = (NSHomeDirectory() as NSString).appendingPathComponent(".config/zen-pwa/skin.json")

    static func load() -> SkinConfig {
        var config = SkinConfig()
        guard let data = FileManager.default.contents(atPath: path),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return config }
        if let layout = json["layout"] as? [String: Any] {
            if let v = layout["separation"] as? Double { config.separation = CGFloat(v) }
            if let v = layout["innerRadius"] as? Double { config.innerRadius = CGFloat(v) }
            if let v = layout["hideDelay"] as? Double { config.hideDelay = max(0.2, v / 1000) }
            if let v = layout["titleBar"] as? Double { config.titleBar = CGFloat(v) }
            if let v = layout["windowRadius"] as? Double { config.windowRadius = CGFloat(v) }
            if let v = layout["lensLift"] as? Double { config.lensLift = CGFloat(v) }
            if let v = layout["borderRadius"] as? Double { config.lensRadius = CGFloat(v) }
            if let v = layout["lensRadius"] as? Double { config.lensRadius = CGFloat(v) }
            if let v = layout["cardRadius"] as? Double { config.cardRadius = CGFloat(v) }
        }
        if let hex = json["defaultFrame"] as? String, let c = color(hex) { config.defaultFrame = c }
        if let apps = json["apps"] as? [String: Any] {
            for (name, value) in apps {
                guard let entry = value as? [String: Any], let hex = entry["frame"] as? String, let c = color(hex) else { continue }
                config.frames[name] = c
                if let id = entry["bundleId"] as? String { config.frames[id] = c }
                if let hex = entry["titleBarColor"] as? String, let sc = color(hex) {
                    config.stripColors[name] = sc
                    if let id = entry["bundleId"] as? String { config.stripColors[id] = sc }
                }
                if let t = entry["titleBar"] as? Double {
                    config.titleBars[name] = CGFloat(t)
                    if let id = entry["bundleId"] as? String { config.titleBars[id] = CGFloat(t) }
                }
            }
        }
        return config
    }

    func titleBar(for app: NSRunningApplication) -> CGFloat {
        if let id = app.bundleIdentifier, let t = titleBars[id] { return t }
        if let name = app.localizedName, let t = titleBars[name] { return t }
        return titleBar
    }

    func stripColor(for app: NSRunningApplication) -> NSColor? {
        if let id = app.bundleIdentifier, let c = stripColors[id] { return c }
        if let name = app.localizedName, let c = stripColors[name] { return c }
        return nil
    }

    func frame(for app: NSRunningApplication) -> NSColor {
        if let id = app.bundleIdentifier, let c = frames[id] { return c }
        if let name = app.localizedName, let c = frames[name] { return c }
        return defaultFrame
    }
}

func color(_ hex: String) -> NSColor? {
    var text = hex.trimmingCharacters(in: .whitespaces)
    if text.hasPrefix("#") { text.removeFirst() }
    guard text.count == 6, let v = Int(text, radix: 16) else { return nil }
    return NSColor(srgbRed: CGFloat((v >> 16) & 255) / 255, green: CGFloat((v >> 8) & 255) / 255, blue: CGFloat(v & 255) / 255, alpha: 1)
}

// MARK: - Geometry

/// Everything is laid out in the overlay's own coordinates (y up). The
/// overlay is larger than the Safari window by `margin` on every side so the
/// backdrop can also cover the window's native drop shadow.
struct LensGeometry: Equatable {
    static let margin: CGFloat = 80

    let bounds: CGRect   // overlay
    let window: CGRect   // the Safari window inside it
    let content: CGRect  // window minus Safari's title bar
    let card: CGRect     // the page, inset by the Zen gap
    let lensRadius: CGFloat
    let cardRadius: CGFloat

    init(size: CGSize, config: SkinConfig) {
        let m = LensGeometry.margin
        bounds = CGRect(origin: .zero, size: size)
        window = bounds.insetBy(dx: m, dy: m)
        // The lens top rises `lensLift` into the hidden title bar (kept within
        // the Zen gap, so none of Safari's bar shows inside the page card).
        let lift = max(0, min(config.lensLift, config.separation))
        let bar = max(0, min(config.titleBar - lift, window.height))
        content = CGRect(x: window.minX, y: window.minY, width: window.width, height: window.height - bar)
        card = content.insetBy(dx: config.separation, dy: config.separation)
        lensRadius = config.lensRadius
        cardRadius = config.cardRadius ?? max(4, config.lensRadius - config.separation * 0.6)
    }

    var strip: CGRect { CGRect(x: window.minX, y: content.maxY, width: window.width, height: window.maxY - content.maxY) }
    var lensPath: CGPath { CGPath(roundedRect: content, cornerWidth: lensRadius, cornerHeight: lensRadius, transform: nil) }
    var cardPath: CGPath { CGPath(roundedRect: card, cornerWidth: cardRadius, cornerHeight: cardRadius, transform: nil) }
}

// MARK: - Rim (drawn on top)

/// The lens itself: one thin Zen rim around the page, with a soft shadow.
/// Without screen access it also paints the title-bar strip as a fallback.
final class RimView: NSView {
    var config = SkinConfig()
    var frameColor = NSColor.gray
    var fallbackStrip: NSColor?     // used only when there is no live backdrop
    var hasBackdrop = false
    var bandOpacity: CGFloat = 1

    override var isFlipped: Bool { false }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.clear.setFill()
        bounds.fill()
        let g = LensGeometry(size: bounds.size, config: config)

        if !hasBackdrop, bandOpacity > 0.001 {
            (fallbackStrip ?? frameColor).withAlphaComponent(bandOpacity).setFill()
            g.strip.fill()
        }

        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow()
        shadow.shadowColor = NSColor(white: 0, alpha: 0.32)
        shadow.shadowBlurRadius = 22
        shadow.shadowOffset = NSSize(width: 0, height: -6)
        shadow.set()
        let rim = NSBezierPath(cgPath: g.lensPath)
        rim.append(NSBezierPath(cgPath: g.cardPath).reversed)
        frameColor.setFill()
        rim.fill()
        NSGraphicsContext.restoreGraphicsState()

        // Hairline on the rim's outer edge, like Zen's window outline.
        NSColor(white: 1, alpha: 0.10).setStroke()
        let edge = NSBezierPath(cgPath: CGPath(roundedRect: g.content.insetBy(dx: 0.5, dy: 0.5),
                                               cornerWidth: g.lensRadius, cornerHeight: g.lensRadius, transform: nil))
        edge.lineWidth = 1
        edge.stroke()
    }
}

// MARK: - Backdrop mask

/// Alpha mask for the backdrop layer: opaque around the window (hiding
/// Safari's title bar, its square corners and its native shadow), feathered
/// out toward the overlay's edge, clear inside the lens, and the title-bar
/// strip scaled by the hover fade.
enum BackdropMask {
    private static var featherCache: (size: CGSize, inner: CGRect, image: CGImage)?

    private static func feather(size: CGSize, inner: CGRect) -> CGImage? {
        if let c = featherCache, c.size == size, c.inner == inner { return c.image }
        let w = max(1, Int(size.width)), h = max(1, Int(size.height))
        let fade = LensGeometry.margin * 0.75
        var alpha = [UInt8](repeating: 0, count: w * h)
        for row in 0..<h {
            let y = CGFloat(h - 1 - row) + 0.5
            let dy = max(inner.minY - y, y - inner.maxY, 0)
            for x in 0..<w {
                let px = CGFloat(x) + 0.5
                let dx = max(inner.minX - px, px - inner.maxX, 0)
                let d = (dx * dx + dy * dy).squareRoot()
                let a = max(0, min(1, 1 - d / fade))
                alpha[row * w + x] = UInt8(a * a * (3 - 2 * a) * 255) // smoothstep
            }
        }
        guard let provider = CGDataProvider(data: Data(alpha) as CFData),
              let image = CGImage(width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 8, bytesPerRow: w,
                                  space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGBitmapInfo(rawValue: 0),
                                  provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent) else { return nil }
        featherCache = (size, inner, image)
        return image
    }

    static func make(_ g: LensGeometry, bandOpacity: CGFloat) -> CGImage? {
        let w = max(1, Int(g.bounds.width)), h = max(1, Int(g.bounds.height))
        guard let base = feather(size: g.bounds.size, inner: g.window.insetBy(dx: -12, dy: -12)),
              let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w,
                                  space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue) else { return nil }
        ctx.draw(base, in: g.bounds)
        ctx.setBlendMode(.copy)
        ctx.setFillColor(gray: bandOpacity, alpha: 1)
        ctx.fill(g.strip)
        ctx.setFillColor(gray: 0, alpha: 1)
        ctx.addPath(g.lensPath)
        ctx.fillPath()
        guard let gray = ctx.makeImage() else { return nil }
        // CALayer masks read alpha, so turn the grey mask into an alpha image.
        guard let actx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
                                   space: CGColorSpaceCreateDeviceRGB(),
                                   bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        actx.clip(to: g.bounds, mask: gray)
        actx.setFillColor(red: 1, green: 1, blue: 1, alpha: 1)
        actx.fill(g.bounds)
        return actx.makeImage()
    }
}

// MARK: - Live backdrop

/// Streams what is behind a window (that app and ZenSkin excluded) into a
/// layer, so the overlay can show the desktop where Safari's title bar,
/// corners and shadow are. Moving wallpapers stay in step. The web app's own
/// window is never captured. Needs Screen Recording.
@available(macOS 14.0, *)
final class BackdropStream: NSObject, SCStreamOutput, SCStreamDelegate {
    private var stream: SCStream?
    private var display: SCDisplay?
    private var rect: CGRect = .zero
    private var starting = false
    private let queue = DispatchQueue(label: "zenskin.backdrop")
    weak var layer: CALayer?
    var onFirstFrame: (() -> Void)?

    static func scale(for display: SCDisplay) -> CGFloat {
        NSScreen.screens.first(where: {
            ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value == display.displayID
        })?.backingScaleFactor ?? 2
    }

    private func configuration(_ rect: CGRect, _ display: SCDisplay) -> SCStreamConfiguration {
        let cfg = SCStreamConfiguration()
        let scale = BackdropStream.scale(for: display)
        cfg.sourceRect = rect.offsetBy(dx: -display.frame.minX, dy: -display.frame.minY)
        cfg.width = max(2, Int(rect.width * scale))
        cfg.height = max(2, Int(rect.height * scale))
        cfg.minimumFrameInterval = CMTime(value: 1, timescale: 60)   // 60 fps
        cfg.pixelFormat = kCVPixelFormatType_32BGRA
        cfg.queueDepth = 4
        cfg.showsCursor = false
        cfg.colorSpaceName = CGColorSpace.sRGB
        return cfg
    }

    /// `rect` is in global top-left (CoreGraphics) coordinates.
    func follow(rect: CGRect, excluding pids: Set<pid_t>) {
        guard CGPreflightScreenCaptureAccess() else { return }
        if let stream, let display, display.frame.contains(CGPoint(x: rect.midX, y: rect.midY)) {
            if rect != self.rect {
                self.rect = rect
                stream.updateConfiguration(configuration(rect, display)) { _ in }
            }
            return
        }
        guard !starting else { return }
        starting = true
        self.rect = rect
        SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: true) { [weak self] content, _ in
            guard let self else { return }
            defer { self.starting = false }
            guard let content,
                  let display = content.displays.first(where: { $0.frame.contains(CGPoint(x: rect.midX, y: rect.midY)) }) else { return }
            let excluded = content.applications.filter { pids.contains($0.processID) }
            let filter = SCContentFilter(display: display, excludingApplications: excluded, exceptingWindows: [])
            let stream = SCStream(filter: filter, configuration: self.configuration(self.rect, display), delegate: self)
            do {
                try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: self.queue)
            } catch { return }
            self.stop()
            self.stream = stream
            self.display = display
            stream.startCapture { _ in }
        }
    }

    func stop() {
        stream?.stopCapture { _ in }
        stream = nil
        display = nil
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, sampleBuffer.isValid,
              let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let raw = attachments.first?[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete,
              let pixels = CMSampleBufferGetImageBuffer(sampleBuffer),
              let surface = CVPixelBufferGetIOSurface(pixels)?.takeUnretainedValue() else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self, let layer = self.layer else { return }
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            layer.contents = surface
            CATransaction.commit()
            if let first = self.onFirstFrame { self.onFirstFrame = nil; first() }
        }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        DispatchQueue.main.async { [weak self] in
            if self?.stream === stream { self?.stream = nil; self?.display = nil }
        }
    }
}

// MARK: - Overlay window

final class SkinWindow: NSPanel {
    let backdropLayer = CALayer()
    let maskLayer = CALayer()
    let rim = RimView()
    var targetID: CGWindowID = 0
    var revealUntil: Date = .distantPast
    var bandOpacity: CGFloat = 1
    var config = SkinConfig()
    private var maskKey = ""
    private var anyStream: AnyObject?

    init() {
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        ignoresMouseEvents = true
        isReleasedWhenClosed = false
        hidesOnDeactivate = false
        level = .normal
        collectionBehavior = [.fullScreenAuxiliary, .ignoresCycle, .transient]

        let root = NSView()
        root.wantsLayer = true
        root.layer?.backgroundColor = .clear
        backdropLayer.contentsGravity = .resize
        backdropLayer.mask = maskLayer
        backdropLayer.isHidden = true
        root.layer?.addSublayer(backdropLayer)
        rim.frame = root.bounds
        rim.autoresizingMask = [.width, .height]
        root.addSubview(rim)
        contentView = root

        if #available(macOS 14.0, *) {
            let stream = BackdropStream()
            stream.layer = backdropLayer
            stream.onFirstFrame = { [weak self] in
                guard let self else { return }
                self.backdropLayer.isHidden = false
                self.rim.hasBackdrop = true
                self.rim.needsDisplay = true
            }
            anyStream = stream
        }
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    // Never pushed below the menu bar: it must sit exactly over the window.
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }

    func layout(target: NSRect, pid: pid_t) {
        let frame = target.insetBy(dx: -LensGeometry.margin, dy: -LensGeometry.margin)
        if !self.frame.equalTo(frame) { setFrame(frame, display: true) }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        backdropLayer.frame = contentView?.bounds ?? .zero
        maskLayer.frame = backdropLayer.bounds
        CATransaction.commit()
        updateMask()

        if #available(macOS 14.0, *), let stream = anyStream as? BackdropStream {
            let primaryHeight = NSScreen.screens.first?.frame.height ?? 0
            let rect = CGRect(x: frame.minX, y: primaryHeight - frame.maxY, width: frame.width, height: frame.height)
            stream.follow(rect: rect, excluding: [pid, ProcessInfo.processInfo.processIdentifier])
        }
    }

    func updateMask() {
        let size = contentView?.bounds.size ?? .zero
        let key = "\(size.width)x\(size.height)|\(config.titleBar)|\(config.separation)|\(config.lensLift)|\(config.lensRadius)|\(config.cardRadius ?? -1)|\(Int(bandOpacity * 100))"
        guard key != maskKey, size.width > 1 else { return }
        maskKey = key
        let g = LensGeometry(size: size, config: config)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        maskLayer.contents = BackdropMask.make(g, bandOpacity: bandOpacity)
        CATransaction.commit()
    }

    func stopBackdrop() {
        if #available(macOS 14.0, *), let stream = anyStream as? BackdropStream { stream.stop() }
        backdropLayer.isHidden = true
        rim.hasBackdrop = false
        if #available(macOS 14.0, *), let stream = anyStream as? BackdropStream {
            stream.onFirstFrame = { [weak self] in
                guard let self else { return }
                self.backdropLayer.isHidden = false
                self.rim.hasBackdrop = true
                self.rim.needsDisplay = true
            }
        }
    }
}

// MARK: - Tracking

struct TargetWindow {
    let id: CGWindowID
    let pid: pid_t
    let frame: NSRect   // Cocoa coordinates
}

final class Skinner: NSObject {
    var config = SkinConfig.load()
    var skins: [CGWindowID: SkinWindow] = [:]
    var timer: Timer?
    var configWatcher: DispatchSourceFileSystemObject?
    let prefix = ProcessInfo.processInfo.environment["ZENSKIN_TARGET"] ?? "com.apple.Safari.WebApp"

    func start() {
        let center = NSWorkspace.shared.notificationCenter
        center.addObserver(self, selector: #selector(appsChanged), name: NSWorkspace.didLaunchApplicationNotification, object: nil)
        center.addObserver(self, selector: #selector(appsChanged), name: NSWorkspace.didTerminateApplicationNotification, object: nil)
        watchConfig()
        appsChanged()
    }

    func targets() -> [NSRunningApplication] {
        NSWorkspace.shared.runningApplications.filter { ($0.bundleIdentifier ?? "").hasPrefix(prefix) }
    }

    @objc func appsChanged() {
        if targets().isEmpty {
            timer?.invalidate()
            timer = nil
            skins.values.forEach { $0.stopBackdrop(); $0.orderOut(nil) }
            skins.removeAll()
        } else if timer == nil {
            // 60 Hz only while a web app is running.
            timer = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in self?.tick() }
            RunLoop.main.add(timer!, forMode: .common)
        }
    }

    func watchConfig() {
        let fd = open(SkinConfig.path, O_EVTONLY)
        guard fd >= 0 else { return }
        let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd, eventMask: [.write, .rename, .delete], queue: .main)
        source.setEventHandler { [weak self] in
            guard let self else { return }
            self.config = SkinConfig.load()
            self.skins.values.forEach { $0.rim.needsDisplay = true }
            if source.data.contains(.rename) || source.data.contains(.delete) {
                source.cancel()
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { self.watchConfig() }
            }
        }
        source.setCancelHandler { close(fd) }
        source.resume()
        configWatcher = source
    }

    func windows(of pids: Set<pid_t>) -> [TargetWindow] {
        guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else { return [] }
        let primaryHeight = NSScreen.screens.first?.frame.height ?? 0
        var out: [TargetWindow] = []
        for info in list {
            guard let pid = info[kCGWindowOwnerPID as String] as? pid_t, pids.contains(pid),
                  (info[kCGWindowLayer as String] as? Int) == 0,
                  let id = info[kCGWindowNumber as String] as? CGWindowID,
                  let b = info[kCGWindowBounds as String] as? [String: CGFloat],
                  let x = b["X"], let y = b["Y"], let w = b["Width"], let h = b["Height"],
                  w > 200, h > 120 else { continue }   // skips Stage Manager thumbnails
            out.append(TargetWindow(id: id, pid: pid, frame: NSRect(x: x, y: primaryHeight - y - h, width: w, height: h)))
        }
        return out
    }

    func isFullscreen(_ frame: NSRect) -> Bool {
        NSScreen.screens.contains { $0.frame.equalTo(frame) }
    }

    func tick() {
        let apps = targets()
        let byPid = Dictionary(apps.map { ($0.processIdentifier, $0) }, uniquingKeysWith: { a, _ in a })
        let found = windows(of: Set(byPid.keys))
        let mouse = NSEvent.mouseLocation
        var live = Set<CGWindowID>()

        for target in found {
            guard let app = byPid[target.pid] else { continue }
            live.insert(target.id)
            let skin = skins[target.id] ?? {
                let s = SkinWindow()
                s.targetID = target.id
                skins[target.id] = s
                return s
            }()

            if isFullscreen(target.frame) {
                if skin.isVisible { skin.stopBackdrop(); skin.orderOut(nil) }
                continue
            }

            var appConfig = config
            appConfig.titleBar = config.titleBar(for: app)
            if skin.config.titleBar != appConfig.titleBar || skin.config.separation != appConfig.separation {
                skin.rim.needsDisplay = true
            }
            skin.config = appConfig
            skin.rim.config = appConfig
            skin.rim.fallbackStrip = config.stripColor(for: app)
            let colour = config.frame(for: app)
            if skin.rim.frameColor != colour {
                skin.rim.frameColor = colour
                skin.rim.needsDisplay = true
            }

            // Zen compact: hovering the (hidden) title bar reveals the real one.
            let topZone = NSRect(x: target.frame.minX, y: target.frame.maxY - appConfig.titleBar - 4,
                                 width: target.frame.width, height: appConfig.titleBar + 10)
            if topZone.contains(mouse) { skin.revealUntil = Date().addingTimeInterval(config.hideDelay) }
            let goal: CGFloat = Date() < skin.revealUntil ? 0 : 1
            var next = skin.bandOpacity + (goal - skin.bandOpacity) * 0.3
            if abs(next - goal) < 0.02 { next = goal }
            if next != skin.bandOpacity {
                skin.bandOpacity = next
                skin.rim.bandOpacity = next
                if !skin.rim.hasBackdrop { skin.rim.needsDisplay = true }
            }

            skin.layout(target: target.frame, pid: target.pid)

            // Sit directly above this window, below whatever is above it.
            skin.order(.above, relativeTo: Int(target.id))
        }

        for (id, skin) in skins where !live.contains(id) {
            skin.stopBackdrop()
            skin.orderOut(nil)
            skins.removeValue(forKey: id)
        }
    }
}

// MARK: - Main

let app = NSApplication.shared
app.setActivationPolicy(.accessory)   // no Dock icon, no menu bar
// Ask once for Screen Recording (live backdrop); ZENSKIN_NO_PROMPT=1 skips it.
if ProcessInfo.processInfo.environment["ZENSKIN_NO_PROMPT"] == nil, !CGPreflightScreenCaptureAccess() {
    _ = CGRequestScreenCaptureAccess()
}
let skinner = Skinner()
skinner.start()
app.run()
