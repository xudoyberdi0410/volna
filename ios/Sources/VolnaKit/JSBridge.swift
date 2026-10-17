import Foundation

/// A JavaScript job from the core (`volna_core_set_js_callback`):
/// `{"script": <source>, "args": <any JSON>}`.
///
/// How the iOS host runs it (one long-lived web view, so state survives
/// between jobs, e.g. a prelude that defines globals):
/// 1. `script` is evaluated as a global script (indirect `eval`), so its
///    top-level declarations become globals.
/// 2. If it evaluates to a function, that function is called with `args`.
/// 3. A promise is awaited.
/// 4. The value is answered as JSON (`undefined` → `null`); a throw or
///    rejection is answered as an error with its message.
///
/// So `{"script": "<prelude>"}` loads code once, `{"script": "volnaJs",
/// "args": "<request>"}` calls it, and `{"script": "1 + 1"}` answers `2`.
public struct JSJob: Equatable, Sendable {
    public let id: UInt64
    public let script: String
    public let args: JSON

    public init?(id: UInt64, request: JSON) {
        guard let script = request["script"].string else { return nil }
        self.id = id
        self.script = script
        args = request["args"]
    }

    /// The body for `WKWebView.callAsyncJavaScript(_:arguments:…)` with the
    /// arguments `script` (string) and `argsJson` (string). It resolves to
    /// the JSON text of the result.
    public static let asyncBody = """
        const value = (0, eval)(script);
        const args = JSON.parse(argsJson);
        let result = typeof value === 'function' ? await value(args) : await value;
        if (result === undefined) result = null;
        return JSON.stringify(result);
        """

    /// The arguments for `asyncBody`.
    public var arguments: [String: String] {
        ["script": script, "argsJson": args.encoded()]
    }

    /// What goes back to the core for the web view's outcome: the JSON text
    /// of the result (checked), or the error message.
    public static func answer(evaluated: Any?, error: Error?) -> Result<String, JSError> {
        if let error {
            return .failure(JSError(JSJob.message(of: error)))
        }
        guard let text = evaluated as? String else {
            // `JSON.stringify` of a function or symbol gives undefined.
            return .success("null")
        }
        guard (try? JSON.parse(text)) != nil else {
            return .failure(JSError("the script result is not JSON"))
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
