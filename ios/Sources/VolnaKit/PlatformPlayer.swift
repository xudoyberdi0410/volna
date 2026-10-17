import Foundation

/// A command from the core for the platform player
/// (`crates/core/src/player/platform.rs`, `PlayerCommand`).
public enum PlayerCommand: Equatable, Sendable {
    /// Prepare `url` paused at `startMs`, then report `ready`.
    case load(url: URL, startMs: Int)
    case play
    case pause
    case seek(positionMs: Int)
    /// Unload; nothing more is reported for the stopped item.
    case stop
    /// 0…1.
    case setVolume(Double)

    public init?(_ j: JSON) {
        switch j["op"].string {
        case "load":
            guard let s = j["url"].string, let url = URL(string: s) else { return nil }
            self = .load(url: url, startMs: j["startMs"].int ?? 0)
        case "play": self = .play
        case "pause": self = .pause
        case "seek": self = .seek(positionMs: j["positionMs"].int ?? 0)
        case "stop": self = .stop
        case "setVolume": self = .setVolume(j["volume"].double ?? 1)
        default: return nil
        }
    }
}

/// What the platform player reports back (`PlayerReport`).
public enum PlayerReport: Equatable, Sendable {
    case ready(durationMs: Int?)
    case playing
    case paused
    case position(ms: Int, durationMs: Int?)
    case buffering(Bool)
    case ended
    case error(message: String, decode: Bool)

    public var json: JSON {
        switch self {
        case .ready(let d): ["type": "ready", "durationMs": JSON(d)]
        case .playing: ["type": "playing"]
        case .paused: ["type": "paused"]
        case .position(let p, let d): ["type": "position", "positionMs": JSON(p), "durationMs": JSON(d)]
        case .buffering(let on): ["type": "buffering", "on": .bool(on)]
        case .ended: ["type": "ended"]
        case .error(let m, let decode): ["type": "error", "message": .string(m), "decode": .bool(decode)]
        }
    }
}

/// The media engine under the bridge: AVPlayer in the app, a fake in tests.
/// Calls come on the bridge's thread (main in the app). The engine tells
/// the bridge what happened with the `token` of the load it is about.
public protocol MediaEngine: AnyObject {
    func load(url: URL, startSeconds: Double, token: Int)
    func play()
    func pause()
    func seek(seconds: Double)
    func stop()
    func setVolume(_ v: Double)
}

/// The platform-player contract between the core and a `MediaEngine`:
/// commands go to the engine; the engine's observations become reports
/// about the latest load only. Stale observations (an item that was
/// stopped or replaced) are dropped, `ready` is sent once per load, and
/// nothing but errors follows `ended`.
///
/// Not thread-safe: use it from one thread (the app: main).
public final class PlatformPlayerBridge {
    public let engine: MediaEngine
    private let report: (PlayerReport) -> Void
    /// The token of the current load; 0 when nothing is loaded.
    public private(set) var token = 0
    private var ready = false
    private var ended = false
    private var playing = false
    private var buffering = false
    private var lastDurationMs: Int?

    public init(engine: MediaEngine, report: @escaping (PlayerReport) -> Void) {
        self.engine = engine
        self.report = report
    }

    public func handle(_ cmd: PlayerCommand) {
        switch cmd {
        case .load(let url, let startMs):
            token += 1
            ready = false
            ended = false
            playing = false
            buffering = false
            lastDurationMs = nil
            engine.load(url: url, startSeconds: Double(startMs) / 1000, token: token)
        case .play:
            engine.play()
        case .pause:
            engine.pause()
        case .seek(let ms):
            ended = false
            engine.seek(seconds: Double(ms) / 1000)
        case .stop:
            token += 1
            ready = false
            playing = false
            engine.stop()
        case .setVolume(let v):
            engine.setVolume(min(1, max(0, v)))
        }
    }

    private func current(_ t: Int) -> Bool { t == token && t != 0 }

    private static func ms(_ seconds: Double?) -> Int? {
        guard let s = seconds, s.isFinite, s > 0 else { return nil }
        return Int((s * 1000).rounded())
    }

    // MARK: - Engine observations

    /// The item can play (duration if known, in seconds).
    public func engineReady(token t: Int, duration: Double?) {
        guard current(t), !ready else { return }
        ready = true
        lastDurationMs = Self.ms(duration)
        report(.ready(durationMs: lastDurationMs))
    }

    /// The duration became known later (live HLS-less streams usually know it at ready).
    public func engineDuration(token t: Int, duration: Double?) {
        guard current(t), let d = Self.ms(duration) else { return }
        lastDurationMs = d
    }

    public func enginePlaying(token t: Int) {
        guard current(t), ready, !ended else { return }
        if buffering {
            buffering = false
            report(.buffering(false))
        }
        if !playing {
            playing = true
            report(.playing)
        }
    }

    public func enginePaused(token t: Int) {
        guard current(t), ready, !ended, playing else { return }
        playing = false
        report(.paused)
    }

    public func engineBuffering(token t: Int, _ on: Bool) {
        guard current(t), ready, !ended, on != buffering else { return }
        buffering = on
        report(.buffering(on))
    }

    /// The periodic playhead (seconds); forwarded while playing.
    public func enginePosition(token t: Int, seconds: Double) {
        guard current(t), ready, !ended, playing, seconds.isFinite, seconds >= 0 else { return }
        report(.position(ms: Int((seconds * 1000).rounded()), durationMs: lastDurationMs))
    }

    public func engineEnded(token t: Int) {
        guard current(t), !ended else { return }
        ended = true
        playing = false
        report(.ended)
    }

    public func engineFailed(token t: Int, message: String, decode: Bool = false) {
        guard current(t) else { return }
        ended = true
        playing = false
        report(.error(message: message, decode: decode))
    }
}
