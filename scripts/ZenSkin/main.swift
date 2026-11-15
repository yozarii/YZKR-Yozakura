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

// MARK: - Drawing

final class SkinView: NSView {
    var config = SkinConfig()
    var frameColor = NSColor.gray
    var stripColor: NSColor?          // Safari's title-bar colour (sampled), so it reads as page
    var bandOpacity: CGFloat = 1 { didSet { needsDisplay = true } }

    override var isFlipped: Bool { false }

    /*
     * Zen compact: one thin rim (the Zen gap) on all four sides around a
     * rounded page card. Safari's title bar falls inside the card's top; it
     * is painted in Safari's own title-bar colour (which Safari takes from
     * the page), so the card looks like page all the way up. Hovering the
     * top edge fades that strip out and the real title bar shows through.
     */
    override func draw(_ dirtyRect: NSRect) {
        NSColor.clear.setFill()
        bounds.fill()

        let sep = config.separation
        let outer = NSBezierPath(roundedRect: bounds, xRadius: config.windowRadius, yRadius: config.windowRadius)
        let card = bounds.insetBy(dx: sep, dy: sep)
        let cardPath = NSBezierPath(roundedRect: card, xRadius: config.innerRadius, yRadius: config.innerRadius)

        NSGraphicsContext.saveGraphicsState()
        outer.addClip()

        let rim = NSBezierPath(rect: bounds)
        rim.append(cardPath.reversed)
        frameColor.setFill()
        rim.fill()

        if bandOpacity > 0.001 {
            NSGraphicsContext.saveGraphicsState()
            cardPath.addClip()
            (stripColor ?? frameColor).withAlphaComponent(bandOpacity).setFill()
            NSRect(x: 0, y: bounds.height - config.titleBar, width: bounds.width, height: config.titleBar).fill()
            NSGraphicsContext.restoreGraphicsState()
        }
        NSGraphicsContext.restoreGraphicsState()
    }
}

/*
 * Samples Safari's title-bar colour: one tiny screenshot of a patch on the
 * right of the bar (clear of the title and traffic lights), averaged, every
 * two seconds. Only the bar is captured, never the page or video. Needs the
 * Screen Recording permission; without it the configured colour is used.
 */
enum TitleBarSampler {
    static func sample(windowID: CGWindowID, width: CGFloat, titleBar: CGFloat, completion: @escaping (NSColor?) -> Void) {
        guard #available(macOS 14.0, *), CGPreflightScreenCaptureAccess() else { completion(nil); return }
        SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: true) { content, _ in
            guard let window = content?.windows.first(where: { $0.windowID == windowID }) else {
                DispatchQueue.main.async { completion(nil) }
                return
            }
            let filter = SCContentFilter(desktopIndependentWindow: window)
            let cfg = SCStreamConfiguration()
            let h = max(4, titleBar - 12)
            cfg.sourceRect = CGRect(x: max(0, width - 140), y: (titleBar - h) / 2, width: 80, height: h)
            cfg.width = 8
            cfg.height = 2
            cfg.showsCursor = false
            SCScreenshotManager.captureImage(contentFilter: filter, configuration: cfg) { image, _ in
                var result: NSColor?
                if let image, let rep = Optional(NSBitmapImageRep(cgImage: image)) {
                    var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, n: CGFloat = 0
                    for y in 0..<rep.pixelsHigh { for x in 0..<rep.pixelsWide {
                        if let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) {
                            r += c.redComponent; g += c.greenComponent; b += c.blueComponent; n += 1
                        }
                    } }
                    if n > 0 { result = NSColor(srgbRed: r / n, green: g / n, blue: b / n, alpha: 1) }
                }
                DispatchQueue.main.async { completion(result) }
            }
        }
    }
}

final class SkinWindow: NSPanel {
    let view = SkinView()
    var targetID: CGWindowID = 0
    var revealUntil: Date = .distantPast
    var nextSample: Date = .distantPast

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
        contentView = view
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
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
            skins.values.forEach { $0.orderOut(nil) }
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
            self.skins.values.forEach { $0.view.needsDisplay = true }
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
                  w > 200, h > 120 else { continue }
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
                skin.orderOut(nil)
                continue
            }

            var appConfig = config
            appConfig.titleBar = config.titleBar(for: app)
            skin.view.config = appConfig
            if skin.view.stripColor == nil, let fallback = config.stripColor(for: app) { skin.view.stripColor = fallback }
            if Date() >= skin.nextSample {
                skin.nextSample = Date().addingTimeInterval(2)
                TitleBarSampler.sample(windowID: target.id, width: target.frame.width, titleBar: appConfig.titleBar) { [weak skin] c in
                    guard let skin, let c else { return }
                    skin.view.stripColor = c
                    skin.view.needsDisplay = true
                }
            }
            let colour = config.frame(for: app)
            if skin.view.frameColor != colour {
                skin.view.frameColor = colour
                skin.view.needsDisplay = true
            }
            if !skin.frame.equalTo(target.frame) {
                skin.setFrame(target.frame, display: true)
            }

            // Zen compact: the top edge reveals the (native) title bar.
            let band = appConfig.titleBar
            let topZone = NSRect(x: target.frame.minX, y: target.frame.maxY - band - 4, width: target.frame.width, height: band + 14)
            if topZone.contains(mouse) { skin.revealUntil = Date().addingTimeInterval(config.hideDelay) }
            let target01: CGFloat = Date() < skin.revealUntil ? 0 : 1
            let next = skin.view.bandOpacity + (target01 - skin.view.bandOpacity) * 0.35
            if abs(next - skin.view.bandOpacity) > 0.002 { skin.view.bandOpacity = abs(next - target01) < 0.01 ? target01 : next }

            // Sit directly above this window, below whatever is above it.
            skin.order(.above, relativeTo: Int(target.id))
        }

        for (id, skin) in skins where !live.contains(id) {
            skin.orderOut(nil)
            skins.removeValue(forKey: id)
        }
    }
}

// MARK: - Main

let app = NSApplication.shared
app.setActivationPolicy(.accessory)   // no Dock icon, no menu bar
// Ask once for Screen Recording (title-bar colour sampling); ZENSKIN_NO_PROMPT=1 skips it.
if ProcessInfo.processInfo.environment["ZENSKIN_NO_PROMPT"] == nil, !CGPreflightScreenCaptureAccess() {
    _ = CGRequestScreenCaptureAccess()
}
let skinner = Skinner()
skinner.start()
app.run()
