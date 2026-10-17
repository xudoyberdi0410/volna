import CVolnaCore
import Foundation

/// A JavaScript job of the stream resolver (`docs/streams.md`, "JsRuntime
/// contract"): `request` is a `JsRequest` as JSON (`{"op":"solve",…}`).
///
/// The host keeps one long-lived web page with `Core.jsPrelude` loaded
/// (it defines `globalThis.volnaJs`), runs `volnaJs(request)` for every job
/// and hands the string the promise resolves to back unchanged. `volnaJs`
/// never rejects; a failure to run at all (no page, WebKit error) is
/// answered as an error instead.
public struct JSJob: Equatable, Sendable {
    public let id: UInt64
    /// The request JSON, passed to `volnaJs` as a string.
    public let request: String

    public init(id: UInt64, request: String) {
        self.id = id
        self.request = request
    }

    /// `op` of the request, for logs.
    public var op: String { (try? JSON.parse(request))?["op"].string ?? "?" }

    /// The body for `WKWebView.callAsyncJavaScript(_:arguments:…)`, with
    /// `arguments` as its arguments.
    public static let asyncBody = "return await volnaJs(r);"

    public var arguments: [String: Any] { ["r": request] }

    /// Is the prelude there? (`callAsyncJavaScript` body; resolves to a Bool.)
    public static let probeBody = "return typeof globalThis.volnaJs === 'function';"

    /// What goes back to the core for the web view's outcome: the resolved
    /// string unchanged, or why it could not run.
    public static func answer(evaluated: Any?, error: Error?) -> Result<String, JSError> {
        if let error {
            return .failure(JSError(JSJob.message(of: error)))
        }
        guard let text = evaluated as? String else {
            return .failure(JSError("volnaJs did not resolve to a string"))
        }
        return .success(text)
    }

    /// A WebKit error's JS message (`WKJavaScriptExceptionMessage`) or its description.
    public static func message(of error: Error) -> String {
        let ns = error as NSError
        if let m = ns.userInfo["WKJavaScriptExceptionMessage"] as? String, !m.isEmpty { return m }
        return ns.localizedDescription
    }
}

public extension Core {
    /// The script a JS context loads once before any job
    /// (`volna_core_js_prelude`: yt-dlp's challenge solver and `volnaJs`).
    static var jsPrelude: String { String(cString: volna_core_js_prelude()) }
}
