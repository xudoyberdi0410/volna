#if os(iOS)
import SwiftUI
import VolnaKit
import WebKit

/// The JS engine of the core's stream resolver (`docs/streams.md`,
/// "JsRuntime contract"): one long-lived page with the core's prelude, where
/// every job runs as `volnaJs(request)`. The page loads no web content (an
/// HTML string), so the web view never talks to YouTube itself; BotGuard
/// only needs a real document. Main thread only.
final class JSHost: NSObject, WKNavigationDelegate {
    let webView: WKWebView
    private var loaded = false
    private var waiting: [(JSJob, (Result<String, JSError>) -> Void)] = []

    override init() {
        let config = WKWebViewConfiguration()
        // The prelude runs as a page script before anything else, again after
        // every reload (the resolver re-sends what it needs).
        let prelude = WKUserScript(source: Core.jsPrelude, injectionTime: .atDocumentStart,
                                   forMainFrameOnly: true)
        config.userContentController.addUserScript(prelude)
        // A separate, non-persistent store: no cookies of the sign-in page here.
        config.websiteDataStore = .nonPersistent()
        webView = WKWebView(frame: CGRect(x: 0, y: 0, width: 2, height: 2), configuration: config)
        super.init()
        webView.navigationDelegate = self
        webView.isUserInteractionEnabled = false
        load()
    }

    private func load() {
        loaded = false
        webView.loadHTMLString("<!doctype html><html><head><meta charset=\"utf-8\"><title>volna</title></head><body></body></html>",
                               baseURL: URL(string: "https://music.youtube.com/"))
    }

    /// Runs one job; `done` gets the string `volnaJs` resolved to, unchanged.
    func run(_ job: JSJob, done: @escaping (Result<String, JSError>) -> Void) {
        guard loaded else {
            waiting.append((job, done))
            return
        }
        webView.callAsyncJavaScript(JSJob.asyncBody, arguments: job.arguments, in: nil, in: .page) { result in
            switch result {
            case .success(let v): done(JSJob.answer(evaluated: v, error: nil))
            case .failure(let e): done(JSJob.answer(evaluated: nil, error: e))
            }
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        loaded = true
        let jobs = waiting
        waiting = []
        for (job, done) in jobs { run(job, done: done) }
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        failWaiting(error)
    }

    /// iOS killed the page's process (memory pressure, background): load it
    /// again; jobs in flight fail and the resolver retries.
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        failWaiting(nil)
        load()
    }

    private func failWaiting(_ error: Error?) {
        let jobs = waiting
        waiting = []
        for (_, done) in jobs {
            done(.failure(JSError(error.map(JSJob.message(of:)) ?? "the JS page was reloaded")))
        }
    }
}

/// Keeps the JS web view in the window (a 2×2 invisible view), so WebKit
/// does not throttle its timers as an off-screen page.
struct JSHostView: UIViewRepresentable {
    let host: JSHost

    func makeUIView(context: Context) -> UIView {
        let v = UIView(frame: CGRect(x: 0, y: 0, width: 2, height: 2))
        v.alpha = 0.01
        v.isUserInteractionEnabled = false
        v.addSubview(host.webView)
        return v
    }

    func updateUIView(_ uiView: UIView, context: Context) {}
}
#endif
