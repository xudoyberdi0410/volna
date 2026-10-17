import Foundation

/// What the screens show about the player, kept from `player_snapshot` and
/// the `player:*` events. A pure value: the app holds one in an observable
/// object and feeds it; decisions stay in the core.
public struct PlayerModel: Sendable {
    public var state = PlaybackState()
    /// Queue entries by uid.
    public var items: [String: PlayerTrack] = [:]
    /// The play order (uids).
    public var order: [String] = []
    /// Index of the current entry in `order`; -1 for none.
    public var index = -1
    /// Queue revision; -1 before the first snapshot.
    public var rev = -1
    public var shuffle = false
    public var `repeat`: RepeatMode = .off
    public var continuation = Continuation()
    public var settings = PlayerSettings()
    public var waveActive = false
    public var sleepEndsAt: Double?
    /// The last failure of any track (also ones skipped past).
    public var lastError: PlayerError?

    public init() {}

    /// Replaces everything with a snapshot.
    public mutating func apply(_ s: PlayerSnapshot) {
        state = s.state
        items = Dictionary(s.queue.items.map { ($0.uid, $0) }, uniquingKeysWith: { _, b in b })
        order = s.queue.items.map(\.uid)
        index = s.queue.index
        rev = s.queue.rev
        shuffle = s.queue.shuffle
        `repeat` = s.queue.repeat
        continuation = s.queue.continuation
        settings = s.settings
        waveActive = s.waveActive
        sleepEndsAt = s.sleepEndsAt
    }

    /// Applies one event. False when the model fell out of sync (a skipped
    /// queue revision or an entry it never saw): fetch a snapshot then.
    @discardableResult
    public mutating func apply(_ e: PlayerEvent) -> Bool {
        switch e {
        case .state(let s):
            state = s
            if let err = s.error { lastError = err }
        case .position(let p):
            guard p.uid == state.uid else { return true }
            state.position = p.position
            state.at = p.at
            state.rate = p.rate
            if let d = p.duration { state.duration = d }
        case .queue(let q):
            if rev >= 0, q.rev <= rev { return true } // stale
            let gap = rev >= 0 && q.rev > rev + 1
            for t in q.added { items[t.uid] = t }
            for t in q.updated { items[t.uid] = t }
            order = q.order
            index = q.index
            rev = q.rev
            shuffle = q.shuffle
            `repeat` = q.repeat
            continuation = q.continuation
            let keep = Set(q.order).union(q.original ?? [])
            items = items.filter { keep.contains($0.key) }
            if gap || order.contains(where: { items[$0] == nil }) { return false }
        case .error(let err):
            lastError = err
        case .settings(let s):
            settings = s
        case .sleep(let endsAt):
            sleepEndsAt = endsAt
        case .wave(let active):
            waveActive = active
        case .queueLow, .journal:
            break
        }
        return true
    }

    /// The entry playing (or loaded).
    public var current: PlayerTrack? {
        if let uid = state.uid, let t = items[uid] { return t }
        guard order.indices.contains(index) else { return nil }
        return items[order[index]]
    }

    /// The queue in play order.
    public var queue: [PlayerTrack] { order.compactMap { items[$0] } }

    /// What plays after the current entry.
    public var upNext: [PlayerTrack] {
        guard index + 1 < order.count else { return [] }
        return order[(index + 1)...].compactMap { items[$0] }
    }

    public var isPlaying: Bool { state.status == .playing }
    public var isLoading: Bool { state.status == .loading || state.buffering }

    /// The playhead at `nowMs` (Unix ms): the last sample, moved on by the
    /// elapsed time while playing, clamped to the duration.
    public func position(atMs nowMs: Double) -> Double {
        var p = state.position
        if state.status == .playing, state.at > 0 {
            p += max(0, nowMs - state.at) / 1000 * state.rate
        }
        if let d = state.duration, d > 0 { p = min(p, d) }
        return max(0, p)
    }
}

public extension Date {
    var unixMs: Double { timeIntervalSince1970 * 1000 }
}
