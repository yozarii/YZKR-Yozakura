import AppKit
import Foundation
import ScreenCaptureKit
import IOKit.pwr_mgt
import Darwin

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
    var titleBar: CGFloat = 32       // title bar height on macOS 26+ (no toolbar)
    var windowRadius: CGFloat = 16
    var topExtra: CGFloat = 0        // top rim edge sits this much higher than the others
    var topDrop: CGFloat = 5         // whole top rim slides down this much, overlapping the page top
    var bottomLift: CGFloat = 11     // whole bottom rim slides up this much, overlapping the page bottom
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
            if let v = layout["borderRadius"] as? Double { config.lensRadius = CGFloat(v) }
            if let v = layout["lensRadius"] as? Double { config.lensRadius = CGFloat(v) }
            if let v = layout["topExtra"] as? Double { config.topExtra = CGFloat(v) }
            if let v = layout["bottomLift"] as? Double { config.bottomLift = CGFloat(v) }
            if let v = layout["topDrop"] as? Double { config.topDrop = CGFloat(v) }
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
    let lens: CGRect     // outer edge of the rim
    let card: CGRect     // the page, inset by the Zen gap
    let lensRadius: CGFloat
    let cardRadius: CGFloat

    init(size: CGSize, config: SkinConfig) {
        let m = LensGeometry.margin
        bounds = CGRect(origin: .zero, size: size)
        window = bounds.insetBy(dx: m, dy: m)
        // The page is exactly the window's visible content (everything below
        // Safari's title bar) and is never covered. The rim sits around it,
        // outside the window on the left, right and bottom, and in the hidden
        // title bar on top.
        let sep = config.separation
        let bar = max(0, min(config.titleBar, window.height))
        content = CGRect(x: window.minX, y: window.minY, width: window.width, height: window.height - bar)
        var c = content
        let cardLift = max(0, min(config.bottomLift, content.height / 4))
        c.origin.y += cardLift
        c.size.height -= cardLift + max(0, min(config.topDrop, content.height / 4))
        card = c
        var l = content.insetBy(dx: -sep, dy: -sep)
        l.size.height += max(0, min(config.topExtra, bar - sep))   // only the top edge moves up
        // The bottom rim keeps its thickness and slides up over the page's
        // bottom edge a little, which tidies the window's own corners.
        let lift = max(0, min(config.bottomLift, content.height / 4))
        l.origin.y += lift
        l.size.height -= lift
        let drop = max(0, min(config.topDrop, content.height / 4))   // top rim keeps its thickness
        l.size.height -= drop
        lens = l
        lensRadius = config.lensRadius
        cardRadius = config.cardRadius ?? max(4, config.lensRadius - sep * 0.75)
    }

    // Safari's title bar above the lens's top rim (shows what is behind).
    var strip: CGRect { CGRect(x: window.minX, y: lens.maxY, width: window.width, height: max(0, window.maxY - lens.maxY)) }
    var lensPath: CGPath { CGPath(roundedRect: lens, cornerWidth: lensRadius, cornerHeight: lensRadius, transform: nil) }
    var cardPath: CGPath { CGPath(roundedRect: card, cornerWidth: cardRadius, cornerHeight: cardRadius, transform: nil) }
}


// MARK: - Feather (9-slice)

/// A small alpha image that is opaque in the middle and fades out over
/// `fade` points to every side. Used as a stretchable (9-slice) mask, so the
/// backdrop's soft edge costs nothing when the window is resized.
enum Feather {
    static let fade: CGFloat = LensGeometry.margin - 12
    static let image: CGImage? = {
        let f = Int(fade)
        let size = f * 2 + 2
        var alpha = [UInt8](repeating: 0, count: size * size * 4)
        for y in 0..<size {
            for x in 0..<size {
                let dx = max(CGFloat(f) - CGFloat(x) - 0.5, CGFloat(x) + 0.5 - CGFloat(f + 2), 0)
                let dy = max(CGFloat(f) - CGFloat(y) - 0.5, CGFloat(y) + 0.5 - CGFloat(f + 2), 0)
                let d = (dx * dx + dy * dy).squareRoot()
                let a = max(0, min(1, 1 - d / fade))
                let v = UInt8(a * a * (3 - 2 * a) * 255)
                let i = (y * size + x) * 4
                alpha[i] = v; alpha[i + 1] = v; alpha[i + 2] = v; alpha[i + 3] = v   // premultiplied white
            }
        }
        guard let provider = CGDataProvider(data: Data(alpha) as CFData) else { return nil }
        return CGImage(width: size, height: size, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: size * 4,
                       space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
                       provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent)
    }()
    static var center: CGRect {
        let size = fade * 2 + 2
        return CGRect(x: fade / size, y: fade / size, width: 2 / size, height: 2 / size)
    }
}

// MARK: - Live backdrop (one stream per display)

/// Streams a whole display, with every skinned web app and ZenSkin itself
/// excluded, into IOSurfaces. Each lens shows its own slice of the frame via
/// `contentsRect`, so moving or resizing a window never restarts capture and
/// the backdrop stays live (moving wallpapers included). Web-app windows,
/// and so their video, are never captured. Needs Screen Recording.
@available(macOS 14.0, *)
final class DisplayBackdrop: NSObject, SCStreamOutput, SCStreamDelegate {
    let displayID: CGDirectDisplayID
    private(set) var frame: CGRect = .zero          // display, global top-left coordinates
    private var stream: SCStream?
    private var excludedPids: Set<pid_t> = []
    private var starting = false
    private let queue = DispatchQueue(label: "zenskin.backdrop")
    var surface: IOSurface?
    var onFrame: ((IOSurface) -> Void)?

    init(displayID: CGDirectDisplayID) { self.displayID = displayID }

    private var logged = false
    func ensure(excluding pids: Set<pid_t>) {
        if !logged, ProcessInfo.processInfo.environment["ZENSKIN_DEBUG"] != nil {
            logged = true
            NSLog("backdrop ensure: access=%d starting=%d stream=%d", CGPreflightScreenCaptureAccess() ? 1 : 0, starting ? 1 : 0, stream == nil ? 0 : 1)
        }
        guard CGPreflightScreenCaptureAccess(), !starting else { return }
        if stream != nil, pids == excludedPids { return }
        starting = true
        SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: true) { [weak self] content, _ in
            guard let self else { return }
            DispatchQueue.main.async {
                defer { self.starting = false }
                guard let content, let display = content.displays.first(where: { $0.displayID == self.displayID }) else {
                    NSLog("backdrop: display %u not found (%d displays)", self.displayID, content?.displays.count ?? -1)
                    return
                }
                let excluded = content.applications.filter { pids.contains($0.processID) }
                let filter = SCContentFilter(display: display, excludingApplications: excluded, exceptingWindows: [])
                self.frame = display.frame
                self.excludedPids = pids
                if let stream = self.stream {
                    stream.updateContentFilter(filter) { _ in }
                    return
                }
                let scale = NSScreen.screens.first(where: {
                    ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value == self.displayID
                })?.backingScaleFactor ?? 2
                let cfg = SCStreamConfiguration()
                cfg.width = Int(display.frame.width * scale)
                cfg.height = Int(display.frame.height * scale)
                cfg.minimumFrameInterval = CMTime(value: 1, timescale: 60)   // 60 fps
                cfg.pixelFormat = kCVPixelFormatType_32BGRA
                cfg.queueDepth = 5
                cfg.showsCursor = false
                cfg.colorSpaceName = CGColorSpace.sRGB
                let stream = SCStream(filter: filter, configuration: cfg, delegate: self)
                guard (try? stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: self.queue)) != nil else { return }
                self.stream = stream
                stream.startCapture { error in
                    if ProcessInfo.processInfo.environment["ZENSKIN_DEBUG"] != nil { NSLog("backdrop stream start: %@", String(describing: error)) }
                }
            }
        }
    }

    /// One still screenshot of the display (same exclusions), for when there
    /// is no frame yet but the live stream must stay off (video playing).
    /// A single screenshot doesn't put macOS into "being recorded" mode the
    /// way a running stream does.
    var still: CGImage?
    private var stillRequested = Date.distantPast
    func takeStill(excluding pids: Set<pid_t>) {
        guard CGPreflightScreenCaptureAccess(), still == nil, surface == nil,
              Date().timeIntervalSince(stillRequested) > 5 else { return }
        stillRequested = Date()
        SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: true) { [weak self] content, _ in
            guard let self, let content,
                  let display = content.displays.first(where: { $0.displayID == self.displayID }) else { return }
            let excluded = content.applications.filter { pids.contains($0.processID) }
            let filter = SCContentFilter(display: display, excludingApplications: excluded, exceptingWindows: [])
            let cfg = SCStreamConfiguration()
            let scale = NSScreen.screens.first(where: {
                ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value == self.displayID
            })?.backingScaleFactor ?? 2
            cfg.width = Int(display.frame.width * scale)
            cfg.height = Int(display.frame.height * scale)
            cfg.showsCursor = false
            SCScreenshotManager.captureImage(contentFilter: filter, configuration: cfg) { image, _ in
                DispatchQueue.main.async {
                    self.frame = display.frame
                    if self.surface == nil, let image { self.still = image }
                }
            }
        }
    }

    /// Stops capturing. The last frame stays on screen (keepFrame) so the
    /// lens edges don't flash while capture is paused for video playback.
    func stop(keepFrame: Bool = false) {
        stream?.stopCapture { _ in }
        stream = nil
        if !keepFrame { surface = nil }
    }
    var isRunning: Bool { stream != nil }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, sampleBuffer.isValid,
              let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let raw = attachments.first?[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete,
              let pixels = CMSampleBufferGetImageBuffer(sampleBuffer),
              let surface = CVPixelBufferGetIOSurface(pixels)?.takeUnretainedValue() else { return }
        let io = unsafeBitCast(surface, to: IOSurface.self)
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            if self.surface == nil, ProcessInfo.processInfo.environment["ZENSKIN_DEBUG"] != nil { NSLog("backdrop first frame %dx%d", IOSurfaceGetWidth(surface), IOSurfaceGetHeight(surface)) }
            self.surface = io
            self.still = nil
            self.onFrame?(io)
        }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        NSLog("backdrop stream stopped: %@", String(describing: error))
        DispatchQueue.main.async { [weak self] in
            if self?.stream === stream { self?.stream = nil }
        }
    }
}

// MARK: - Overlay window (all Core Animation, nothing redrawn per frame)

final class SkinWindow: NSPanel {
    let backdropHolder = CALayer()     // masked: everything except lens and title strip
    let holeMask = CAShapeLayer()
    let backdrop = CALayer()           // slice of the display stream, feathered
    let featherMask = CALayer()
    let strip = CALayer()              // backdrop over Safari's title bar (hover fades it)
    let fallbackStrip = CALayer()      // when there is no screen access
    let shadowHost = CALayer()         // lens shadow, masked so it never falls on the page
    let shadowMask = CAShapeLayer()
    let shadowShape = CAShapeLayer()
    let rim = CAShapeLayer()           // the Zen rim
    let edge = CAShapeLayer()          // hairline on the rim's outer edge

    var targetID: CGWindowID = 0
    var displayID: CGDirectDisplayID = 0
    var revealUntil: Date = .distantPast
    var bandOpacity: CGFloat = 1
    var config = SkinConfig()
    var hasBackdrop = false
    private var geometryKey = ""

    init() {
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        ignoresMouseEvents = true          // every click reaches Safari
        isReleasedWhenClosed = false
        hidesOnDeactivate = false
        level = .normal
        collectionBehavior = [.fullScreenAuxiliary, .ignoresCycle, .transient]
        animationBehavior = .none

        let root = NSView()
        root.wantsLayer = true
        let layer = root.layer!
        layer.backgroundColor = .clear

        holeMask.fillRule = .evenOdd
        backdropHolder.mask = holeMask
        featherMask.contents = Feather.image
        featherMask.contentsCenter = Feather.center
        backdrop.mask = featherMask
        backdrop.contentsGravity = .resize
        strip.contentsGravity = .resize
        backdropHolder.addSublayer(backdrop)
        backdropHolder.isHidden = true
        strip.isHidden = true

        rim.fillRule = .evenOdd
        // The shadow lives on its own layer, masked to everything outside the
        // page: a shadow on the rim itself (shaped like the whole lens) showed
        // through the page and darkened it. Even all round, so the top
        // corners don't look flat.
        shadowShape.shadowColor = NSColor.black.cgColor
        shadowShape.shadowOpacity = 0.30
        shadowShape.shadowRadius = 12
        shadowShape.shadowOffset = CGSize(width: 0, height: -1.5)
        shadowMask.fillRule = .evenOdd
        shadowHost.mask = shadowMask
        shadowHost.addSublayer(shadowShape)
        edge.fillColor = nil
        edge.strokeColor = NSColor(white: 1, alpha: 0.10).cgColor
        edge.lineWidth = 1

        for l in [backdropHolder, strip, fallbackStrip, shadowHost, rim, edge] as [CALayer] { layer.addSublayer(l) }
        contentView = root
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    // Never pushed below the menu bar: it must sit exactly over the window.
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }

    /// Layers created by hand default to 1x; draw everything at the screen's
    /// real scale so the rim and its edges stay sharp on Retina.
    private var appliedScale: CGFloat = 0
    func syncScale() {
        let scale = screen?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
        guard scale != appliedScale else { return }
        appliedScale = scale
        for l in [backdropHolder, holeMask, backdrop, featherMask, strip, fallbackStrip, shadowHost, shadowMask, shadowShape, rim, edge] as [CALayer] {
            l.contentsScale = scale
            l.rasterizationScale = scale
        }
    }

    func noActions(_ body: () -> Void) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        body()
        CATransaction.commit()
    }

    /// Lays the lens out over `target` (Cocoa coordinates).
    func place(over target: NSRect, frameColor: NSColor, fallback: NSColor?) {
        let frame = target.insetBy(dx: -LensGeometry.margin, dy: -LensGeometry.margin)
        if !self.frame.equalTo(frame) { setFrame(frame, display: false) }
        syncScale()
        let size = frame.size
        let key = "\(size.width)x\(size.height)|\(config.titleBar)|\(config.separation)|\(config.lensRadius)|\(config.topExtra)|\(config.bottomLift)|\(config.topDrop)|\(config.cardRadius ?? -1)"
        noActions {
            if key != geometryKey {
                geometryKey = key
                let g = LensGeometry(size: size, config: config)
                let holes = CGMutablePath()
                holes.addRect(g.bounds)
                holes.addPath(g.lensPath)
                holes.addRect(g.strip)
                holeMask.path = holes
                for l in [backdropHolder, holeMask, backdrop, featherMask] as [CALayer] { l.frame = g.bounds }
                strip.frame = g.strip
                fallbackStrip.frame = g.strip
                let rimPath = CGMutablePath()
                rimPath.addPath(g.lensPath)
                rimPath.addPath(g.cardPath)
                rim.path = rimPath
                for l in [shadowHost, shadowMask, shadowShape] as [CALayer] { l.frame = g.bounds }
                shadowShape.path = g.lensPath
                shadowShape.shadowPath = g.lensPath
                let outsidePage = CGMutablePath()
                outsidePage.addRect(g.bounds)
                outsidePage.addPath(g.cardPath)
                shadowMask.path = outsidePage
                edge.path = CGPath(roundedRect: g.lens.insetBy(dx: 0.5, dy: 0.5), cornerWidth: g.lensRadius, cornerHeight: g.lensRadius, transform: nil)
            }
            rim.fillColor = frameColor.cgColor
            shadowShape.fillColor = frameColor.cgColor
            fallbackStrip.backgroundColor = (fallback ?? frameColor).cgColor
            strip.opacity = Float(bandOpacity)
            fallbackStrip.opacity = hasBackdrop ? 0 : Float(bandOpacity)
        }
    }

    /// Points both backdrop layers at this window's slice of the display frame.
    /// `live`: the frame is from the running stream. A frozen frame (video
    /// playing) is only used for the hidden title bar; the soft area around
    /// the lens then shows the real screen, so nothing that moved since the
    /// freeze (e.g. Stage Manager thumbnails) appears stale or doubled.
    func show(surface: Any, displayFrame: CGRect, live: Bool = true) {
        let primaryHeight = NSScreen.screens.first?.frame.height ?? 0
        let f = frame
        // Overlay in global top-left coordinates, then in the display's unit space.
        let top = CGRect(x: f.minX, y: primaryHeight - f.maxY, width: f.width, height: f.height)
        func unit(_ r: CGRect) -> CGRect {
            // contentsRect's unit space starts at the bottom-left on macOS.
            CGRect(x: (r.minX - displayFrame.minX) / displayFrame.width,
                   y: 1 - (r.maxY - displayFrame.minY) / displayFrame.height,
                   width: r.width / displayFrame.width,
                   height: r.height / displayFrame.height)
        }
        let g = LensGeometry(size: f.size, config: config)
        // Strip in global top-left coordinates.
        let s = CGRect(x: f.minX + g.strip.minX, y: primaryHeight - (f.minY + g.strip.maxY), width: g.strip.width, height: g.strip.height)
        noActions {
            backdrop.contents = surface
            backdrop.contentsRect = unit(top)
            strip.contents = surface
            strip.contentsRect = unit(s)
            if !hasBackdrop {
                hasBackdrop = true
                strip.isHidden = false
                fallbackStrip.opacity = 0
            }
            backdropHolder.isHidden = !live
        }
    }

    func dropBackdrop() {
        noActions {
            hasBackdrop = false
            backdropHolder.isHidden = true
            strip.isHidden = true
            fallbackStrip.opacity = Float(bandOpacity)
        }
    }
}

// MARK: - Playback guard

var backdropPausedForVideo = false   // reported in zenskin-status.json

/// macOS treats protected (FairPlay) video as being recorded while any
/// screen capture runs, and Netflix then misbehaves, even though the web
/// app's own window is excluded. Video playback holds a "keep the display
/// awake" power assertion, so while one is held by WebKit / Safari / a
/// skinned web app, the live backdrop is paused (last frame stays).
enum Playback {
    static func processName(_ pid: pid_t) -> String {
        var buffer = [CChar](repeating: 0, count: 1024)
        return proc_name(pid, &buffer, UInt32(buffer.count)) > 0 ? String(cString: buffer) : ""
    }

    static func isPlaying(webAppPids: Set<pid_t>) -> Bool {
        var raw: Unmanaged<CFDictionary>?
        guard IOPMCopyAssertionsByProcess(&raw) == kIOReturnSuccess,
              let byPid = raw?.takeRetainedValue() as? [NSNumber: [[String: Any]]] else { return false }
        for (pidNumber, assertions) in byPid {
            let pid = pid_t(pidNumber.int32Value)
            let keepsDisplayOn = assertions.contains {
                let type = ($0["AssertType"] as? String) ?? ""
                return type == "PreventUserIdleDisplaySleep" || type == "NoDisplaySleepAssertion"
            }
            guard keepsDisplayOn else { continue }
            if webAppPids.contains(pid) { return true }
            let name = processName(pid)
            if name.contains("WebKit") || name.contains("Safari") || name.contains("WebApp") { return true }
        }
        return false
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
    var known: [CGWindowID: pid_t] = [:]
    var lastScan: Date = .distantPast
    var displayLink: AnyObject?
    var fallbackTimer: Timer?
    var configWatcher: DispatchSourceFileSystemObject?
    var backdrops: [CGDirectDisplayID: AnyObject] = [:]
    var playing = false
    var lastPlaybackCheck: Date = .distantPast
    var quietSince: Date = .distantPast
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
        lastScan = .distantPast
        if targets().isEmpty {
            stopTicking()
            skins.values.forEach { $0.orderOut(nil) }
            skins.removeAll()
            known.removeAll()
            if #available(macOS 14.0, *) { backdrops.values.forEach { ($0 as? DisplayBackdrop)?.stop() } }
            backdrops.removeAll()
        } else {
            startTicking()
        }
    }

    // Tick in step with the display (120 Hz on ProMotion), so the lens
    // follows a dragged or resized window without trailing behind.
    func startTicking() {
        guard displayLink == nil, fallbackTimer == nil else { return }
        if #available(macOS 14.0, *), let screen = NSScreen.main {
            let link = screen.displayLink(target: self, selector: #selector(tick))
            link.add(to: .main, forMode: .common)
            displayLink = link
        } else {
            let timer = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in self?.tick() }
            RunLoop.main.add(timer, forMode: .common)
            fallbackTimer = timer
        }
    }

    func stopTicking() {
        if #available(macOS 14.0, *) { (displayLink as? CADisplayLink)?.invalidate() }
        displayLink = nil
        fallbackTimer?.invalidate()
        fallbackTimer = nil
    }

    func watchConfig() {
        let fd = open(SkinConfig.path, O_EVTONLY)
        guard fd >= 0 else { return }
        let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd, eventMask: [.write, .rename, .delete], queue: .main)
        source.setEventHandler { [weak self] in
            guard let self else { return }
            self.config = SkinConfig.load()
            if source.data.contains(.rename) || source.data.contains(.delete) {
                source.cancel()
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { self.watchConfig() }
            }
        }
        source.setCancelHandler { close(fd) }
        source.resume()
        configWatcher = source
    }

    /// Full window scan (finds new windows) a few times a second; between
    /// scans only the known windows are queried, which is much cheaper.
    func currentWindows(pids: Set<pid_t>) -> [TargetWindow] {
        let primaryHeight = NSScreen.screens.first?.frame.height ?? 0
        var infos: [[String: Any]] = []
        if Date().timeIntervalSince(lastScan) > 0.25 || known.isEmpty {
            lastScan = Date()
            infos = (CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]]) ?? []
        } else {
            // This API wants the raw window IDs stored as pointer values.
            var ptrs: [UnsafeRawPointer?] = known.keys.map { UnsafeRawPointer(bitPattern: UInt($0)) }
            let ids = CFArrayCreate(nil, &ptrs, ptrs.count, nil)
            infos = (CGWindowListCreateDescriptionFromArray(ids) as? [[String: Any]]) ?? []
            if infos.isEmpty {
                lastScan = Date()
                infos = (CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]]) ?? []
            }
        }
        var out: [TargetWindow] = []
        for info in infos {
            guard let pid = info[kCGWindowOwnerPID as String] as? pid_t, pids.contains(pid),
                  (info[kCGWindowLayer as String] as? Int) == 0,
                  (info[kCGWindowIsOnscreen as String] as? Bool) ?? true,
                  let id = info[kCGWindowNumber as String] as? CGWindowID,
                  let b = info[kCGWindowBounds as String] as? [String: CGFloat],
                  let x = b["X"], let y = b["Y"], let w = b["Width"], let h = b["Height"],
                  w > 200, h > 120 else { continue }   // skips Stage Manager thumbnails
            out.append(TargetWindow(id: id, pid: pid, frame: NSRect(x: x, y: primaryHeight - y - h, width: w, height: h)))
        }
        known = Dictionary(out.map { ($0.id, $0.pid) }, uniquingKeysWith: { a, _ in a })
        return out
    }

    func displayID(for frame: NSRect) -> CGDirectDisplayID {
        let mid = CGPoint(x: frame.midX, y: frame.midY)
        let screen = NSScreen.screens.first(where: { $0.frame.contains(mid) }) ?? NSScreen.main
        return (screen?.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? CGMainDisplayID()
    }

    func isFullscreen(_ frame: NSRect) -> Bool {
        NSScreen.screens.contains { $0.frame.equalTo(frame) }
    }

    var ticks = 0
    @objc func tick() {
        ticks += 1
        if ProcessInfo.processInfo.environment["ZENSKIN_DEBUG"] != nil, ticks % 120 == 1 {
            NSLog("tick %d skins %d", ticks, skins.count)
        }
        let apps = targets()
        guard !apps.isEmpty else { return }
        let byPid = Dictionary(apps.map { ($0.processIdentifier, $0) }, uniquingKeysWith: { a, _ in a })
        let pids = Set(byPid.keys)

        // Pause the live backdrop while video plays; resume 2 s after it stops.
        if Date().timeIntervalSince(lastPlaybackCheck) > 0.25 {
            lastPlaybackCheck = Date()
            let now = Playback.isPlaying(webAppPids: pids)
            if now {
                quietSince = .distantFuture
                if !playing {
                    playing = true
                    backdropPausedForVideo = true
                    if #available(macOS 14.0, *) { backdrops.values.forEach { ($0 as? DisplayBackdrop)?.stop(keepFrame: true) } }
                }
            } else if playing {
                if quietSince == .distantFuture { quietSince = Date() }
                if Date().timeIntervalSince(quietSince) > 2 { playing = false; backdropPausedForVideo = false }
            }
        }

        let found = currentWindows(pids: pids)
        let mouse = NSEvent.mouseLocation
        var live = Set<CGWindowID>()
        var usedDisplays = Set<CGDirectDisplayID>()

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
                if skin.isVisible { skin.orderOut(nil) }
                continue
            }

            var appConfig = config
            appConfig.titleBar = config.titleBar(for: app)
            skin.config = appConfig

            // Zen compact: hovering the (hidden) title bar reveals the real one.
            let topZone = NSRect(x: target.frame.minX, y: target.frame.maxY - appConfig.titleBar - 4,
                                 width: target.frame.width, height: appConfig.titleBar + 10)
            if topZone.contains(mouse) { skin.revealUntil = Date().addingTimeInterval(config.hideDelay) }
            let goal: CGFloat = Date() < skin.revealUntil ? 0 : 1
            var next = skin.bandOpacity + (goal - skin.bandOpacity) * 0.25
            if abs(next - goal) < 0.02 { next = goal }
            skin.bandOpacity = next

            skin.place(over: target.frame, frameColor: config.frame(for: app), fallback: config.stripColor(for: app))
            skin.order(.above, relativeTo: Int(target.id))

            let did = displayID(for: target.frame)
            skin.displayID = did
            usedDisplays.insert(did)
            if #available(macOS 14.0, *) {
                let backdrop = (backdrops[did] as? DisplayBackdrop) ?? {
                    let b = DisplayBackdrop(displayID: did)
                    b.onFrame = { [weak self] surface in self?.deliver(surface, display: did) }
                    backdrops[did] = b
                    return b
                }()
                if !playing { backdrop.ensure(excluding: pids.union([ProcessInfo.processInfo.processIdentifier])) }
                if let surface = backdrop.surface {
                    skin.show(surface: surface, displayFrame: backdrop.frame, live: backdrop.isRunning)
                } else if let still = backdrop.still {
                    skin.show(surface: still, displayFrame: backdrop.frame, live: false)
                } else if playing {
                    backdrop.takeStill(excluding: pids.union([ProcessInfo.processInfo.processIdentifier]))
                }
            }
        }

        for (id, skin) in skins where !live.contains(id) {
            skin.orderOut(nil)
            skins.removeValue(forKey: id)
        }
        if #available(macOS 14.0, *) {
            for (did, b) in backdrops where !usedDisplays.contains(did) {
                (b as? DisplayBackdrop)?.stop()
                backdrops.removeValue(forKey: did)
            }
        }
    }

    @available(macOS 14.0, *)
    func deliver(_ surface: IOSurface, display: CGDirectDisplayID) {
        guard let b = backdrops[display] as? DisplayBackdrop else { return }
        for skin in skins.values where skin.displayID == display && skin.isVisible {
            skin.show(surface: surface, displayFrame: b.frame)
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
// Status for zen-skin / humans: is the live backdrop allowed?
func writeStatus() {
    let path = (NSHomeDirectory() as NSString).appendingPathComponent(".config/zen-pwa/zenskin-status.json")
    let status: [String: Any] = ["screenRecording": CGPreflightScreenCaptureAccess(), "pid": ProcessInfo.processInfo.processIdentifier,
                                 "backdropPausedForVideo": backdropPausedForVideo,
                                 "updated": ISO8601DateFormatter().string(from: Date())]
    if let data = try? JSONSerialization.data(withJSONObject: status, options: [.prettyPrinted]) {
        try? data.write(to: URL(fileURLWithPath: path))
    }
}
writeStatus()
Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { _ in writeStatus() }
let skinner = Skinner()
skinner.start()
app.run()
