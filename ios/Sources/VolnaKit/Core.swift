import CVolnaCore
import Foundation

/// How the app starts the core (`volna_core_init`).
public struct CoreConfig: Encodable, Sendable {
    /// Databases, settings, the queue. Must survive app updates and reinstalls:
    /// on iOS, Application Support inside the app container.
    public var dataDir: String
    /// Covers; the OS may clear it. Default `<dataDir>/cache`.
    public var cacheDir: String?
    /// Serve audio on `http://127.0.0.1:<port>/` for AVPlayer.
    public var audioServer: Bool = true
    /// A `tracing` filter for stderr, e.g. `"info"`.
    public var log: String?
    /// `"aac"`: stream URLs ask for AAC in m4a only (AVPlayer has no Opus/WebM).
    public var codec: String? = "aac"
    /// Debug builds of the core only: route every request through this proxy.
    public var devProxy: String?

    public init(dataDir: String, cacheDir: String? = nil, audioServer: Bool = true,
                log: String? = nil, codec: String? = "aac", devProxy: String? = nil) {
        self.dataDir = dataDir
        self.cacheDir = cacheDir
        self.audioServer = audioServer
        self.log = log
        self.codec = codec
        self.devProxy = devProxy
    }
}

/// A failed command. The core answers `"<kind>: <message>"`
/// (`bad_args`, `not_found`, `upstream`, `internal`, `unsupported`, `busy`, …).
public struct CoreError: Error, Equatable, CustomStringConvertible, Sendable {
    public let kind: String
    public let message: String

    public init(kind: String, message: String) {
        self.kind = kind
        self.message = message
    }

    /// Splits `"<kind>: <message>"`; a text without a kind is `other`.
    public init(wire: String) {
        if let r = wire.range(of: ": "),
           wire[..<r.lowerBound].allSatisfy({ $0.isLetter || $0 == "_" }),
           !wire[..<r.lowerBound].isEmpty
        {
            self.init(kind: String(wire[..<r.lowerBound]), message: String(wire[r.upperBound...]))
        } else {
            self.init(kind: "other", message: wire)
        }
    }

    public var description: String { "\(kind): \(message)" }
}

/// Something the core sent without being asked.
public struct CoreEvent: Sendable {
    public let name: String
    public let payload: JSON

    public init(name: String, payload: JSON) {
        self.name = name
        self.payload = payload
    }
}

/// A `volna://` response (cover images).
public struct ProtoResponse: Sendable {
    public let status: Int
    public let headers: [(String, String)]
    public let body: Data

    public func header(_ name: String) -> String? {
        headers.first { $0.0.caseInsensitiveCompare(name) == .orderedSame }?.1
    }
}

/// What the app needs from the core: commands, events, images. `Core`
/// is the real one; tests use fakes.
public protocol CoreTransport: AnyObject, Sendable {
    /// Runs `cmd` and returns its JSON result, or throws `CoreError`.
    func call(_ cmd: String, _ args: JSON) async throws -> JSON
    func proto(_ uri: String) async -> ProtoResponse
    /// Every event, as long as the stream is iterated.
    func events() -> AsyncStream<CoreEvent>
}

/// The Rust core behind the C ABI (`volna_core.h`).
///
/// Replies, events, player commands and JS jobs arrive on core threads;
/// replies resume the awaiting task, the rest goes to the handlers and
/// event streams, which must not block. Every string the core hands over is
/// copied and freed right away (`volna_core_free_string` /
/// `volna_core_free_bytes`).
public final class Core: CoreTransport, @unchecked Sendable {
    private let handle: OpaquePointer
    private let lock = NSLock()
    private var nextId: UInt64 = 0
    private var replies: [UInt64: CheckedContinuation<JSON, Error>] = [:]
    private var protos: [UInt64: CheckedContinuation<ProtoResponse, Never>] = [:]
    private var listeners: [UUID: AsyncStream<CoreEvent>.Continuation] = [:]
    private var playerHandler: (@Sendable (JSON) -> Void)?
    private var jsHandler: (@Sendable (UInt64, JSON) -> Void)?
    private var closed = false
    /// The `user_data` of every callback: an unretained pointer to `self`,
    /// valid until `shutdown` (callbacks are cleared there first).
    private var me: UnsafeMutableRawPointer { Unmanaged.passUnretained(self).toOpaque() }

    /// Starts the core; throws `CoreError(kind: "init")` with the core's reason.
    public init(config: CoreConfig) throws {
        let json = String(decoding: try Wire.encoder.encode(config), as: UTF8.self)
        guard let h = volna_core_init(json) else {
            let reason = Core.take(volna_core_last_error()) ?? "unknown"
            throw CoreError(kind: "init", message: reason)
        }
        handle = h
        volna_core_set_reply_callback(h, { user, id, ok, json in
            Core.from(user).onReply(id, ok, Core.take(json) ?? "null")
        }, me)
        volna_core_set_event_callback(h, { user, name, payload in
            Core.from(user).onEvent(Core.take(name) ?? "", Core.take(payload) ?? "null")
        }, me)
        volna_core_set_proto_callback(h, { user, id, status, headers, body, len in
            let data = body.map { Data(bytes: $0, count: len) } ?? Data()
            volna_core_free_bytes(body, len)
            Core.from(user).onProto(id, Int(status), Core.take(headers) ?? "[]", data)
        }, me)
        volna_core_set_player_callback(h, { user, cmd in
            Core.from(user).onPlayer(Core.take(cmd) ?? "null")
        }, me)
        volna_core_set_js_callback(h, { user, id, request in
            Core.from(user).onJs(id, Core.take(request) ?? "null")
        }, me)
    }

    deinit {
        shutdown()
    }

    private static func from(_ user: UnsafeMutableRawPointer?) -> Core {
        Unmanaged<Core>.fromOpaque(user!).takeUnretainedValue()
    }

    /// Copies and frees a string the core handed over.
    private static func take(_ p: UnsafeMutablePointer<CChar>?) -> String? {
        guard let p else { return nil }
        defer { volna_core_free_string(p) }
        return String(cString: p)
    }

    private func newId() -> UInt64 {
        nextId += 1
        return nextId
    }

    // MARK: - Commands

    public func call(_ cmd: String, _ args: JSON = .null) async throws -> JSON {
        let argsText = args.isNull ? nil : args.encoded()
        return try await withCheckedThrowingContinuation { cont in
            lock.lock()
            if closed {
                lock.unlock()
                cont.resume(throwing: CoreError(kind: "closed", message: "the core was shut down"))
                return
            }
            let id = newId()
            replies[id] = cont
            lock.unlock()
            if let argsText {
                volna_core_call(handle, id, cmd, argsText)
            } else {
                volna_core_call(handle, id, cmd, nil)
            }
        }
    }

    private func onReply(_ id: UInt64, _ ok: Bool, _ text: String) {
        lock.lock()
        let cont = replies.removeValue(forKey: id)
        lock.unlock()
        guard let cont else { return }
        let value = (try? JSON.parse(text)) ?? .string(text)
        if ok {
            cont.resume(returning: value)
        } else {
            cont.resume(throwing: CoreError(wire: value.string ?? text))
        }
    }

    // MARK: - Images

    public func proto(_ uri: String) async -> ProtoResponse {
        await withCheckedContinuation { cont in
            lock.lock()
            if closed {
                lock.unlock()
                cont.resume(returning: ProtoResponse(status: 503, headers: [], body: Data()))
                return
            }
            let id = newId()
            protos[id] = cont
            lock.unlock()
            volna_core_proto(handle, id, uri)
        }
    }

    private func onProto(_ id: UInt64, _ status: Int, _ headers: String, _ body: Data) {
        lock.lock()
        let cont = protos.removeValue(forKey: id)
        lock.unlock()
        let pairs = (try? JSON.parse(headers))?.array?.compactMap { p -> (String, String)? in
            guard let k = p[0].string, let v = p[1].string else { return nil }
            return (k, v)
        } ?? []
        cont?.resume(returning: ProtoResponse(status: status, headers: pairs, body: body))
    }

    // MARK: - Events

    public func events() -> AsyncStream<CoreEvent> {
        AsyncStream(bufferingPolicy: .bufferingNewest(512)) { cont in
            let key = UUID()
            lock.lock()
            listeners[key] = cont
            lock.unlock()
            cont.onTermination = { [weak self] _ in
                guard let self else { return }
                self.lock.lock()
                self.listeners.removeValue(forKey: key)
                self.lock.unlock()
            }
        }
    }

    private func onEvent(_ name: String, _ payload: String) {
        let event = CoreEvent(name: name, payload: (try? JSON.parse(payload)) ?? .null)
        lock.lock()
        let targets = Array(listeners.values)
        lock.unlock()
        for t in targets { t.yield(event) }
    }

    // MARK: - Platform player

    /// Where player commands go (`{"op":"load",…}`); see `PlayerCommand`.
    public func setPlayerHandler(_ f: (@Sendable (JSON) -> Void)?) {
        lock.lock()
        playerHandler = f
        lock.unlock()
    }

    private func onPlayer(_ text: String) {
        lock.lock()
        let f = playerHandler
        lock.unlock()
        f?((try? JSON.parse(text)) ?? .null)
    }

    /// What the platform player did (`PlayerReport`). False when the core
    /// did not take it.
    @discardableResult
    public func report(_ r: PlayerReport) -> Bool {
        lock.lock()
        let isClosed = closed
        lock.unlock()
        if isClosed { return false }
        return volna_core_player_report(handle, r.json.encoded())
    }

    // MARK: - JavaScript

    /// Where JS jobs go: `(request id, {"script":…,"args":…})`. Answer with
    /// `jsResult`.
    public func setJsHandler(_ f: (@Sendable (UInt64, JSON) -> Void)?) {
        lock.lock()
        jsHandler = f
        lock.unlock()
    }

    private func onJs(_ id: UInt64, _ text: String) {
        lock.lock()
        let f = jsHandler
        lock.unlock()
        let request = (try? JSON.parse(text)) ?? .null
        if let f {
            f(id, request)
        } else {
            _ = jsResult(id, .failure("no JS runtime"))
        }
    }

    /// The answer to JS job `id`: the script's JSON result text, or an error
    /// message. False when nothing waits for that id.
    @discardableResult
    public func jsResult(_ id: UInt64, _ result: Result<String, JSError>) -> Bool {
        lock.lock()
        let isClosed = closed
        lock.unlock()
        if isClosed { return false }
        switch result {
        case .success(let json):
            return volna_core_js_result(handle, id, true, json)
        case .failure(let e):
            return volna_core_js_result(handle, id, false, JSON.string(e.message).encoded())
        }
    }

    // MARK: - Lifecycle

    /// Saves the player and stops the core. Pending calls fail with
    /// `closed`. Not from inside a handler. Idempotent.
    public func shutdown() {
        lock.lock()
        if closed {
            lock.unlock()
            return
        }
        closed = true
        lock.unlock()
        // Clears the callbacks, then waits for the core to stop: nothing
        // calls back into `self` after this.
        volna_core_shutdown(handle)
        lock.lock()
        let pending = replies
        let images = protos
        let streams = Array(listeners.values)
        replies = [:]
        protos = [:]
        listeners = [:]
        playerHandler = nil
        jsHandler = nil
        lock.unlock()
        for c in pending.values {
            c.resume(throwing: CoreError(kind: "closed", message: "the core was shut down"))
        }
        for c in images.values { c.resume(returning: ProtoResponse(status: 503, headers: [], body: Data())) }
        for s in streams { s.finish() }
    }
}

/// A JS job failure (the message goes back to the core).
public struct JSError: Error, Equatable, Sendable {
    public let message: String
    public init(_ message: String) { self.message = message }
}

extension Result where Failure == JSError {
    static func failure(_ message: String) -> Self { .failure(JSError(message)) }
}
