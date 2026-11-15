// YZKR AI-OS — native single-window shell around the local AI-OS server.
//
// - Starts `uv run ai-os/server.py` if it isn't already serving on 127.0.0.1:7777 (and stops it on quit).
// - One WKWebView window, hidden title bar, Zen-like chrome rendered by the web UI.
// - yzkr:// URL scheme (yzkr://palette, yzkr://editor?path=…, yzkr://graph?focus=…, yzkr://chat?q=…, yzkr://home).
// - ⌥Space global hotkey (Carbon, no Accessibility permission) → bring window forward with the ⌘K palette.
// - Closing the window hides it; the app stays resident for the hotkey and URL scheme.

import AppKit
import Carbon.HIToolbox
import WebKit

let home = FileManager.default.homeDirectoryForCurrentUser
let yzkr = ProcessInfo.processInfo.environment["YZKR_HOME"].map { URL(fileURLWithPath: $0) } ?? home.appendingPathComponent("YZKR")
let base = URL(string: "http://127.0.0.1:7777/")!
let uvCandidates = ["/opt/homebrew/bin/uv", "/usr/local/bin/uv", home.appendingPathComponent(".local/bin/uv").path]

/// Transparent strip over the hidden title bar: drags the window, double-click zooms (web content can't do this in WKWebView).
final class DragStrip: NSView {
    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2 { window?.performZoom(nil) } else { window?.performDrag(with: event) }
    }
    override var mouseDownCanMoveWindow: Bool { true }
}

final class AppDelegate: NSObject, NSApplicationDelegate, WKNavigationDelegate, WKUIDelegate, NSWindowDelegate {
    var window: NSWindow!
    var web: WKWebView!
    var server: Process?
    var loaded = false
    var pendingURLs: [String] = []
    var hotKeyRef: EventHotKeyRef?

    // Build the window before launch finishes: a yzkr:// cold launch delivers open(urls:) ahead of didFinishLaunching.
    func applicationWillFinishLaunching(_ note: Notification) {
        buildMenu()
        buildWindow()
    }

    func applicationDidFinishLaunching(_ note: Notification) {
        registerHotKey()
        ensureServer { [weak self] ok in
            guard let self else { return }
            if ok { self.web.load(URLRequest(url: base)) } else { self.showError() }
        }
    }

    // ───────── window ─────────
    func buildWindow() {
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1320, height: 840),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                          backing: .buffered, defer: false)
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.title = "YZKR AI-OS"
        window.minSize = NSSize(width: 860, height: 520)
        window.backgroundColor = NSColor(srgbRed: 0x0b / 255, green: 0x0b / 255, blue: 0x10 / 255, alpha: 1)
        window.appearance = NSAppearance(named: .darkAqua)
        window.delegate = self
        window.isReleasedWhenClosed = false
        window.tabbingMode = .disallowed
        window.setFrameAutosaveName("YZKRAIOSMain")
        if !window.setFrameUsingName("YZKRAIOSMain") { window.center() }

        let cfg = WKWebViewConfiguration()
        cfg.preferences.setValue(true, forKey: "developerExtrasEnabled")
        cfg.websiteDataStore = .nonPersistent()
        web = WKWebView(frame: .zero, configuration: cfg)
        web.navigationDelegate = self
        web.uiDelegate = self
        web.setValue(false, forKey: "drawsBackground")
        web.allowsBackForwardNavigationGestures = false

        let root = NSView()
        web.translatesAutoresizingMaskIntoConstraints = false
        let strip = DragStrip()
        strip.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(web)
        root.addSubview(strip)
        NSLayoutConstraint.activate([
            web.leadingAnchor.constraint(equalTo: root.leadingAnchor), web.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            web.topAnchor.constraint(equalTo: root.topAnchor), web.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            // leave the right side of the title bar (status pill) to the page; 40pt matches the web titlebar height
            strip.leadingAnchor.constraint(equalTo: root.leadingAnchor), strip.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -180),
            strip.topAnchor.constraint(equalTo: root.topAnchor), strip.heightAnchor.constraint(equalToConstant: 40),
        ])
        window.contentView = root
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    func showWindow() {
        guard let window else { return }
        if window.isMiniaturized { window.deminiaturize(nil) }
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool { sender.orderOut(nil); return false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool { showWindow(); return true }

    func showError() {
        let html = """
        <body style="background:#0b0b10;color:#e6e6ee;font:14px -apple-system;padding:60px">
        <h2 style="color:#f87171">AI-OS server didn't start</h2>
        <p>Check <code>~/YZKR/ai-os/data/server.log</code>, or run <code>uv run ~/YZKR/ai-os/server.py</code> in a terminal.</p></body>
        """
        web.loadHTMLString(html, baseURL: nil)
    }

    // ───────── server lifecycle ─────────
    func ping(_ done: @escaping (Bool) -> Void) {
        var req = URLRequest(url: base.appendingPathComponent("api/health"))
        req.timeoutInterval = 1
        URLSession.shared.dataTask(with: req) { _, resp, _ in
            done((resp as? HTTPURLResponse).map { $0.statusCode == 401 || $0.statusCode == 200 } ?? false)  // 401 = up, token-guarded
        }.resume()
    }

    func ensureServer(_ done: @escaping (Bool) -> Void) {
        ping { [weak self] up in
            if up { return DispatchQueue.main.async { done(true) } }
            DispatchQueue.main.async { self?.launchServer(); self?.waitForServer(tries: 60, done) }
        }
    }

    func launchServer() {
        guard let uv = uvCandidates.first(where: { FileManager.default.isExecutableFile(atPath: $0) }) else { return }
        let dataDir = yzkr.appendingPathComponent("ai-os/data")
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        let logURL = dataDir.appendingPathComponent("server.log")
        FileManager.default.createFile(atPath: logURL.path, contents: nil)
        let log = try? FileHandle(forWritingTo: logURL)
        let p = Process()
        p.executableURL = URL(fileURLWithPath: uv)
        p.arguments = ["run", "-q", yzkr.appendingPathComponent("ai-os/server.py").path]
        p.currentDirectoryURL = yzkr
        var env = ProcessInfo.processInfo.environment
        env["PATH"] = "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        p.environment = env
        p.standardOutput = log
        p.standardError = log
        try? p.run()
        server = p
    }

    func waitForServer(tries: Int, _ done: @escaping (Bool) -> Void) {
        ping { [weak self] up in
            DispatchQueue.main.async {
                if up { return done(true) }
                if tries <= 0 || self?.server?.isRunning == false { return done(false) }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { self?.waitForServer(tries: tries - 1, done) }
            }
        }
    }

    func applicationWillTerminate(_ note: Notification) {
        if let s = server, s.isRunning { s.terminate() }  // only the server we started
    }

    // ───────── URL scheme ─────────
    func application(_ application: NSApplication, open urls: [URL]) {
        showWindow()
        for url in urls where url.scheme == "yzkr" { route(url.absoluteString) }
    }

    func route(_ url: String) {
        guard loaded else { pendingURLs.append(url); return }
        deliver(url, attempts: 50)
    }

    /// ES modules can finish after WKWebView's didFinish, so retry until the page has installed window.YZKR.
    func deliver(_ url: String, attempts: Int) {
        guard let data = try? JSONSerialization.data(withJSONObject: [url]), let arg = String(data: data, encoding: .utf8) else { return }
        web.evaluateJavaScript("window.YZKR ? (YZKR.openURL(\(arg)[0]), true) : false") { [weak self] r, _ in
            if (r as? Bool) != true && attempts > 0 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { self?.deliver(url, attempts: attempts - 1) }
            }
        }
    }

    // ───────── global hotkey: ⌥Space ─────────
    func registerHotKey() {
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(), { _, _, userData in
            let me = Unmanaged<AppDelegate>.fromOpaque(userData!).takeUnretainedValue()
            DispatchQueue.main.async { me.showWindow(); me.route("yzkr://palette") }
            return noErr
        }, 1, &spec, Unmanaged.passUnretained(self).toOpaque(), nil)
        let id = EventHotKeyID(signature: OSType(0x595A_4B52), id: 1)  // 'YZKR'
        RegisterEventHotKey(UInt32(kVK_Space), UInt32(optionKey), id, GetApplicationEventTarget(), 0, &hotKeyRef)
    }

    // ───────── navigation policy ─────────
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        loaded = webView.url?.host == "127.0.0.1"
        let queued = pendingURLs
        pendingURLs = []
        queued.forEach(route)
    }

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = action.request.url else { return decisionHandler(.cancel) }
        if url.host == "127.0.0.1" && url.port == 7777 || url.scheme == "about" { return decisionHandler(.allow) }
        if ["http", "https", "mailto"].contains(url.scheme ?? "") { NSWorkspace.shared.open(url) }
        decisionHandler(.cancel)
    }

    // target=_blank links
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for action: WKNavigationAction,
                 windowFeatures: WKWindowFeatures) -> WKWebView? {
        if let url = action.request.url, ["http", "https"].contains(url.scheme ?? "") { NSWorkspace.shared.open(url) }
        return nil
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) { webView.reload() }

    // ───────── JS dialogs (confirm/prompt used by the editor & macros) ─────────
    func webView(_ webView: WKWebView, runJavaScriptAlertPanelWithMessage message: String, initiatedByFrame frame: WKFrameInfo,
                 completionHandler: @escaping () -> Void) {
        let a = NSAlert(); a.messageText = message; a.runModal(); completionHandler()
    }

    func webView(_ webView: WKWebView, runJavaScriptConfirmPanelWithMessage message: String, initiatedByFrame frame: WKFrameInfo,
                 completionHandler: @escaping (Bool) -> Void) {
        let a = NSAlert(); a.messageText = message; a.addButton(withTitle: "OK"); a.addButton(withTitle: "Cancel")
        completionHandler(a.runModal() == .alertFirstButtonReturn)
    }

    func webView(_ webView: WKWebView, runJavaScriptTextInputPanelWithPrompt prompt: String, defaultText: String?,
                 initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping (String?) -> Void) {
        let a = NSAlert(); a.messageText = prompt; a.addButton(withTitle: "OK"); a.addButton(withTitle: "Cancel")
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 320, height: 24)); field.stringValue = defaultText ?? ""
        a.accessoryView = field
        a.window.initialFirstResponder = field
        completionHandler(a.runModal() == .alertFirstButtonReturn ? field.stringValue : nil)
    }

    // ───────── menu (Edit menu is required for ⌘C/⌘V/⌘A inside WKWebView) ─────────
    func buildMenu() {
        let main = NSMenu()
        let appItem = NSMenuItem(); main.addItem(appItem)
        let app = NSMenu()
        app.addItem(withTitle: "About YZKR AI-OS", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        app.addItem(.separator())
        app.addItem(withTitle: "Reload", action: #selector(reload), keyEquivalent: "r")
        app.addItem(withTitle: "Hide YZKR AI-OS", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        app.addItem(.separator())
        app.addItem(withTitle: "Quit YZKR AI-OS", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = app

        let editItem = NSMenuItem(); main.addItem(editItem)
        let edit = NSMenu(title: "Edit")
        edit.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        edit.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "Z")
        edit.addItem(.separator())
        edit.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        editItem.submenu = edit

        let winItem = NSMenuItem(); main.addItem(winItem)
        let win = NSMenu(title: "Window")
        win.addItem(withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        win.addItem(withTitle: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
        win.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        winItem.submenu = win
        NSApp.windowsMenu = win
        NSApp.mainMenu = main
    }

    @objc func reload() { web.reload() }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
