import Cocoa
import WebKit

class ZenPWAWindow: NSWindow {
    override var canBecomeKey: Bool { return true }
    override var canBecomeMain: Bool { return true }
}

class AppDelegate: NSObject, NSApplicationDelegate, NSTextFieldDelegate, WKNavigationDelegate {
    var window: ZenPWAWindow!
    var webView: WKWebView!
    var toolbarView: NSVisualEffectView!
    var sidebarView: NSVisualEffectView!
    var urlField: NSTextField!
    var pinButton: NSButton!

    var rootURL: URL?
    var isToolbarVisible = false
    var isSidebarPinned = false
    var isWindowPinnedOnTop = false
    var mouseMonitor: Any?

    init(initialURL: URL? = nil) {
        self.rootURL = initialURL
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        guard let incoming = urls.first,
              let components = URLComponents(url: incoming, resolvingAgainstBaseURL: false),
              let targetStr = components.queryItems?.first(where: { $0.name == "url" })?.value,
              let targetURL = URL(string: targetStr) else { return }
        if rootURL == nil { rootURL = targetURL }
        loadTarget(targetURL)
    }

    func loadTarget(_ url: URL) {
        if webView != nil {
            webView.load(URLRequest(url: url))
            urlField?.stringValue = url.absoluteString
        } else {
            rootURL = url
        }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        setupMenuShortcuts()

        let screenRect = NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1280, height: 800)
        let winRect = NSRect(x: screenRect.midX - 580, y: screenRect.midY - 360, width: 1160, height: 720)

        window = ZenPWAWindow(
            contentRect: winRect,
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.isOpaque = false
        window.backgroundColor = NSColor(red: 0.09, green: 0.09, blue: 0.11, alpha: 1.0)
        setTrafficLights(hidden: true)

        let contentView = window.contentView!

        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        config.allowsAirPlayForMediaPlayback = true
        config.mediaTypesRequiringUserActionForPlayback = []

        webView = WKWebView(frame: contentView.bounds, configuration: config)
        webView.autoresizingMask = [.width, .height]
        webView.navigationDelegate = self
        webView.setValue(false, forKey: "drawsBackground")
        webView.customUserAgent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15"
        contentView.addSubview(webView)

        let tbHeight: CGFloat = 42
        toolbarView = NSVisualEffectView(frame: NSRect(x: 0, y: contentView.bounds.height - tbHeight, width: contentView.bounds.width, height: tbHeight))
        toolbarView.autoresizingMask = [.width, .minYMargin]
        toolbarView.material = .hudWindow
        toolbarView.blendingMode = .withinWindow
        toolbarView.state = .active
        toolbarView.alphaValue = 0.0
        toolbarView.isHidden = true

        let backBtn = makeIconButton(symbol: "chevron.left", tip: "Back", action: #selector(goBack), x: 80)
        let fwdBtn = makeIconButton(symbol: "chevron.right", tip: "Forward", action: #selector(goForward), x: 110)
        let reloadBtn = makeIconButton(symbol: "arrow.clockwise", tip: "Reload", action: #selector(reloadPage), x: 140)
        let sidebarBtn = makeIconButton(symbol: "sidebar.left", tip: "Toggle Tool Sidebar (Cmd+S)", action: #selector(toggleSidebar), x: 174)

        urlField = NSTextField(frame: NSRect(x: 214, y: 9, width: contentView.bounds.width - 260, height: 24))
        urlField.autoresizingMask = [.width]
        urlField.bezelStyle = .roundedBezel
        urlField.font = NSFont.systemFont(ofSize: 12)
        urlField.placeholderString = "Enter URL..."
        urlField.delegate = self

        let copyBtn = makeIconButton(symbol: "link", tip: "Copy Current URL", action: #selector(copyCurrentURL), x: contentView.bounds.width - 38)
        copyBtn.autoresizingMask = [.minXMargin]

        toolbarView.addSubview(backBtn)
        toolbarView.addSubview(fwdBtn)
        toolbarView.addSubview(reloadBtn)
        toolbarView.addSubview(sidebarBtn)
        toolbarView.addSubview(urlField)
        toolbarView.addSubview(copyBtn)
        contentView.addSubview(toolbarView)

        let sbWidth: CGFloat = 46
        sidebarView = NSVisualEffectView(frame: NSRect(x: 8, y: (contentView.bounds.height - 250) / 2, width: sbWidth, height: 250))
        sidebarView.autoresizingMask = [.minYMargin, .maxYMargin]
        sidebarView.material = .hudWindow
        sidebarView.blendingMode = .withinWindow
        sidebarView.state = .active
        sidebarView.wantsLayer = true
        sidebarView.layer?.cornerRadius = 12
        sidebarView.layer?.borderWidth = 0.5
        sidebarView.layer?.borderColor = NSColor.white.withAlphaComponent(0.15).cgColor
        sidebarView.alphaValue = 0.0
        sidebarView.isHidden = true

        let homeBtn = makeSidebarButton(symbol: "house", tip: "App Home", action: #selector(goHome), y: 204)
        pinButton = makeSidebarButton(symbol: "pin", tip: "Always on Top", action: #selector(toggleAlwaysOnTop), y: 162)
        let zoomInBtn = makeSidebarButton(symbol: "plus.magnifyingglass", tip: "Zoom In", action: #selector(zoomIn), y: 120)
        let zoomOutBtn = makeSidebarButton(symbol: "minus.magnifyingglass", tip: "Zoom Out", action: #selector(zoomOut), y: 78)
        let openZenBtn = makeSidebarButton(symbol: "safari", tip: "Open Current Page in Zen", action: #selector(openInZen), y: 36)

        sidebarView.addSubview(homeBtn)
        sidebarView.addSubview(pinButton)
        sidebarView.addSubview(zoomInBtn)
        sidebarView.addSubview(zoomOutBtn)
        sidebarView.addSubview(openZenBtn)
        contentView.addSubview(sidebarView)

        if let closeBtn = window.standardWindowButton(.closeButton)?.superview {
            closeBtn.wantsLayer = true
            closeBtn.layer?.zPosition = 999
        }

        mouseMonitor = NSEvent.addLocalMonitorForEvents(matching: [.mouseMoved, .leftMouseDragged]) { [weak self] event in
            self?.handleMouseLocation()
            return event
        }

        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        if let url = rootURL {
            urlField.stringValue = url.absoluteString
            webView.load(URLRequest(url: url))
        }
    }

    func handleMouseLocation() {
        guard let win = window, win.isKeyWindow else { return }
        let loc = win.mouseLocationOutsideOfEventStream
        let height = win.contentView?.bounds.height ?? 720

        let nearTop = loc.y >= (height - 38) && loc.y <= height && loc.x >= 0 && loc.x <= win.frame.width
        let urlFocused = (win.firstResponder as? NSTextView)?.delegate as? NSTextField == urlField

        if nearTop || urlFocused {
            setToolbar(visible: true)
        } else if isToolbarVisible && loc.y < (height - 52) {
            setToolbar(visible: false)
        }

        if !isSidebarPinned {
            let nearLeft = loc.x >= 0 && loc.x <= 18 && loc.y > 80 && loc.y < (height - 80)
            let insideSidebar = sidebarView.frame.insetBy(dx: -12, dy: -12).contains(loc) && !sidebarView.isHidden
            if nearLeft || insideSidebar {
                setSidebar(visible: true)
            } else if !sidebarView.isHidden {
                setSidebar(visible: false)
            }
        }
    }

    func setToolbar(visible: Bool) {
        guard isToolbarVisible != visible else { return }
        isToolbarVisible = visible
        setTrafficLights(hidden: !visible)
        if visible { toolbarView.isHidden = false }
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = 0.16
            toolbarView.animator().alphaValue = visible ? 1.0 : 0.0
        }, completionHandler: {
            if !self.isToolbarVisible { self.toolbarView.isHidden = true }
        })
    }

    func setSidebar(visible: Bool) {
        if visible { sidebarView.isHidden = false }
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = 0.16
            sidebarView.animator().alphaValue = visible ? 1.0 : 0.0
        }, completionHandler: {
            if !visible && !self.isSidebarPinned { self.sidebarView.isHidden = true }
        })
    }

    func setTrafficLights(hidden: Bool) {
        window.standardWindowButton(.closeButton)?.isHidden = hidden
        window.standardWindowButton(.miniaturizeButton)?.isHidden = hidden
        window.standardWindowButton(.zoomButton)?.isHidden = hidden
    }

    func makeIconButton(symbol: String, tip: String, action: Selector, x: CGFloat) -> NSButton {
        let btn = NSButton(frame: NSRect(x: x, y: 8, width: 26, height: 26))
        btn.bezelStyle = .texturedRounded
        btn.isBordered = false
        btn.image = NSImage(systemSymbolName: symbol, accessibilityDescription: tip)
        btn.contentTintColor = .labelColor
        btn.toolTip = tip
        btn.target = self
        btn.action = action
        return btn
    }

    func makeSidebarButton(symbol: String, tip: String, action: Selector, y: CGFloat) -> NSButton {
        let btn = NSButton(frame: NSRect(x: 7, y: y, width: 32, height: 32))
        btn.bezelStyle = .texturedRounded
        btn.isBordered = false
        btn.image = NSImage(systemSymbolName: symbol, accessibilityDescription: tip)
        btn.contentTintColor = .labelColor
        btn.toolTip = tip
        btn.target = self
        btn.action = action
        return btn
    }

    @objc func goBack() { webView.goBack() }
    @objc func goForward() { webView.goForward() }
    @objc func reloadPage() { webView.reload() }
    @objc func goHome() { if let url = rootURL { webView.load(URLRequest(url: url)) } }

    @objc func toggleSidebar() {
        isSidebarPinned.toggle()
        setSidebar(visible: isSidebarPinned)
    }

    @objc func focusURLBar() {
        setToolbar(visible: true)
        window.makeFirstResponder(urlField)
    }

    @objc func copyCurrentURL() {
        guard let current = webView.url?.absoluteString else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(current, forType: .string)
    }

    @objc func toggleAlwaysOnTop() {
        isWindowPinnedOnTop.toggle()
        window.level = isWindowPinnedOnTop ? .floating : .normal
        pinButton.contentTintColor = isWindowPinnedOnTop ? .systemBlue : .labelColor
    }

    @objc func zoomIn() { webView.pageZoom += 0.1 }
    @objc func zoomOut() { webView.pageZoom = max(0.5, webView.pageZoom - 0.1) }

    @objc func openInZen() {
        guard let current = webView.url else { return }
        let zenURL = URL(fileURLWithPath: "/Applications/Zen Browser.app")
        let fallbackZen = URL(fileURLWithPath: "/Applications/Zen.app")
        let appURL = FileManager.default.fileExists(atPath: zenURL.path) ? zenURL : fallbackZen
        NSWorkspace.shared.open([current], withApplicationAt: appURL, configuration: NSWorkspace.OpenConfiguration())
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        if commandSelector == #selector(NSResponder.insertNewline(_:)) {
            var str = urlField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            if !str.contains("://") { str = "https://" + str }
            if let url = URL(string: str) {
                webView.load(URLRequest(url: url))
                window.makeFirstResponder(webView)
                setToolbar(visible: false)
            }
            return true
        }
        return false
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        if let current = webView.url?.absoluteString {
            urlField.stringValue = current
        }
    }

    func setupMenuShortcuts() {
        let mainMenu = NSMenu()
        let appMenuItem = NSMenuItem()
        mainMenu.addItem(appMenuItem)
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "Focus Address Bar", action: #selector(focusURLBar), keyEquivalent: "l")
        appMenu.addItem(withTitle: "Toggle Tool Sidebar", action: #selector(toggleSidebar), keyEquivalent: "s")
        appMenu.addItem(withTitle: "Reload", action: #selector(reloadPage), keyEquivalent: "r")
        appMenu.addItem(withTitle: "Zoom In", action: #selector(zoomIn), keyEquivalent: "+")
        appMenu.addItem(withTitle: "Zoom Out", action: #selector(zoomOut), keyEquivalent: "-")
        appMenu.addItem(NSMenuItem.separator())
        appMenu.addItem(withTitle: "Quit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appMenuItem.submenu = appMenu
        NSApp.mainMenu = mainMenu
    }
}

let args = CommandLine.arguments
var startURL: URL? = nil
if args.count > 1, let parsed = URL(string: args[1]), parsed.scheme != nil {
    startURL = parsed
}

let app = NSApplication.shared
let delegate = AppDelegate(initialURL: startURL)
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
