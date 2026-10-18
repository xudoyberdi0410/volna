import Foundation

// `crates/player/src/model.rs` as the core sends it (camelCase JSON,
// lowercase enums); `docs/native-player.md` §10. DSP and the visualizer
// are dropped on mobile and not modelled.

public enum Origin: String, Codable, Sendable {
    case user, wave, radio, autoplay

    public init(from d: Decoder) throws {
        self = Origin(rawValue: try d.singleValueContainer().decode(String.self)) ?? .user
    }
}

public enum RepeatMode: String, Codable, Sendable, CaseIterable {
    case off, all, one

    public init(from d: Decoder) throws {
        self = RepeatMode(rawValue: try d.singleValueContainer().decode(String.self)) ?? .off
    }

    public var next: RepeatMode {
        switch self {
        case .off: .all
        case .all: .one
        case .one: .off
        }
    }
}

public enum PlayStatus: String, Codable, Sendable {
    case idle, loading, playing, paused, ended

    public init(from d: Decoder) throws {
        self = PlayStatus(rawValue: try d.singleValueContainer().decode(String.self)) ?? .idle
    }
}

/// `model::TrackKind` (not the ytm one).
public enum PlayerTrackKind: String, Codable, Sendable {
    case track, video

    public init(from d: Decoder) throws {
        self = PlayerTrackKind(rawValue: try d.singleValueContainer().decode(String.self)) ?? .track
    }
}

public enum ContinuationKind: String, Codable, Sendable {
    case none, wave, radio, ui

    public init(from d: Decoder) throws {
        self = ContinuationKind(rawValue: try d.singleValueContainer().decode(String.self)) ?? .none
    }
}

public struct PArtist: Codable, Hashable, Sendable {
    public var id: String?
    public var name: String

    public init(id: String?, name: String) {
        self.id = id
        self.name = name
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(String.self, forKey: .id)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
    }
}

public struct PAlbum: Codable, Hashable, Sendable {
    public var id: String?
    public var title: String

    public init(id: String?, title: String) {
        self.id = id
        self.title = title
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(String.self, forKey: .id)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
    }
}

/// What `player_set_queue` / `player_enqueue` take.
public struct TrackInput: Encodable, Hashable, Sendable {
    public var id: String
    public var title: String
    public var artists: [PArtist]
    public var album: PAlbum?
    /// Seconds.
    public var duration: Double?
    public var cover: String?
    public var explicit: Bool
    public var kind: PlayerTrackKind
    public var origin: Origin?

    public init(id: String, title: String, artists: [PArtist] = [], album: PAlbum? = nil,
                duration: Double? = nil, cover: String? = nil, explicit: Bool = false,
                kind: PlayerTrackKind = .track, origin: Origin? = nil)
    {
        self.id = id
        self.title = title
        self.artists = artists
        self.album = album
        self.duration = duration
        self.cover = cover
        self.explicit = explicit
        self.kind = kind
        self.origin = origin
    }

    /// From a catalog track; the cover is the largest thumbnail.
    public init(_ t: Track, origin: Origin? = nil) {
        self.init(
            id: t.videoId,
            title: t.title,
            artists: t.artists.map { PArtist(id: $0.id, name: $0.name) },
            album: t.album.map { PAlbum(id: $0.id, title: $0.name) },
            duration: t.durationS.map(Double.init),
            cover: t.thumbnails.best?.url,
            explicit: t.explicit,
            kind: t.kind == .song ? .track : .video,
            origin: origin
        )
    }
}

/// A queue entry.
public struct PlayerTrack: Decodable, Hashable, Sendable, Identifiable {
    public var uid: String
    /// The videoId.
    public var trackId: String
    public var title: String
    public var artists: [PArtist]
    public var album: PAlbum?
    public var duration: Double?
    public var cover: String?
    public var explicit: Bool
    public var kind: PlayerTrackKind
    public var origin: Origin
    public var unavailable: Bool

    public var id: String { uid }

    enum CodingKeys: String, CodingKey {
        case uid, id, title, artists, album, duration, cover, explicit, kind, origin, unavailable
    }

    public init(uid: String, trackId: String, title: String, artists: [PArtist] = [],
                album: PAlbum? = nil, duration: Double? = nil, cover: String? = nil,
                explicit: Bool = false, kind: PlayerTrackKind = .track, origin: Origin = .user,
                unavailable: Bool = false)
    {
        self.uid = uid
        self.trackId = trackId
        self.title = title
        self.artists = artists
        self.album = album
        self.duration = duration
        self.cover = cover
        self.explicit = explicit
        self.kind = kind
        self.origin = origin
        self.unavailable = unavailable
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        uid = try c.decode(String.self, forKey: .uid)
        trackId = try c.decode(String.self, forKey: .id)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        artists = try c.decodeIfPresent([PArtist].self, forKey: .artists) ?? []
        album = try c.decodeIfPresent(PAlbum.self, forKey: .album)
        duration = try c.decodeIfPresent(Double.self, forKey: .duration)
        cover = try c.decodeIfPresent(String.self, forKey: .cover)
        explicit = try c.decodeIfPresent(Bool.self, forKey: .explicit) ?? false
        kind = try c.decodeIfPresent(PlayerTrackKind.self, forKey: .kind) ?? .track
        origin = try c.decodeIfPresent(Origin.self, forKey: .origin) ?? .user
        unavailable = try c.decodeIfPresent(Bool.self, forKey: .unavailable) ?? false
    }

    public var artistNames: String { artists.map(\.name).filter { !$0.isEmpty }.joined(separator: ", ") }

    /// Back to an input (to like it, add it to a playlist, …).
    public var input: TrackInput {
        TrackInput(id: trackId, title: title, artists: artists, album: album, duration: duration,
                   cover: cover, explicit: explicit, kind: kind, origin: origin)
    }

    /// As a ytm `Track` (what `library_like` and playlists take).
    public var asTrack: Track {
        Track(videoId: trackId, title: title,
              artists: artists.map { ArtistRef(id: $0.id, name: $0.name) },
              album: album.map { AlbumRef(id: $0.id, name: $0.title) },
              durationS: duration.map { Int($0.rounded()) },
              thumbnails: cover.map { [Thumbnail(url: $0)] } ?? [],
              explicit: explicit, kind: kind == .track ? .song : .video)
    }
}

/// `StreamInfo`; `quality` is `HIGH` / `LOW`.
public struct StreamInfo: Decodable, Hashable, Sendable {
    public var codec: String?
    public var bitrateKbps: Double?
    public var quality: String?
    public var mime: String?
}

/// `PlayerError`. `kind`: bot_check, unavailable, no_yt_dlp, decode,
/// timeout, other; `action`: skipped, stopped, retrying, none.
public struct PlayerError: Decodable, Hashable, Sendable {
    public var uid: String?
    public var trackId: String?
    public var kind: String
    public var message: String
    public var action: String
    public var streak: Int

    enum CodingKeys: String, CodingKey { case uid, trackId, kind, message, action, streak }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        uid = try c.decodeIfPresent(String.self, forKey: .uid)
        trackId = try c.decodeIfPresent(String.self, forKey: .trackId)
        kind = try c.decodeIfPresent(String.self, forKey: .kind) ?? "other"
        message = try c.decodeIfPresent(String.self, forKey: .message) ?? ""
        action = try c.decodeIfPresent(String.self, forKey: .action) ?? "none"
        streak = try c.decodeIfPresent(Int.self, forKey: .streak) ?? 0
    }

    /// For people.
    public var text: String {
        switch kind {
        case "bot_check": "YouTube просит подтвердить, что вы не бот"
        case "unavailable": "Трек недоступен"
        case "no_yt_dlp": "Нет источника звука"
        case "decode": "Не удалось декодировать звук"
        case "timeout": "Превышено время ожидания"
        default: message.isEmpty ? "Ошибка воспроизведения" : message
        }
    }
}

/// `PlaybackState` (also the `player:state` payload).
public struct PlaybackState: Decodable, Hashable, Sendable {
    public var status: PlayStatus = .idle
    public var uid: String?
    public var trackId: String?
    /// Seconds, sampled at `at`.
    public var position: Double = 0
    /// Unix ms of the position sample.
    public var at: Double = 0
    public var duration: Double?
    public var rate: Double = 1
    public var buffering = false
    public var volume: Double = 0.7
    public var muted = false
    public var stream: StreamInfo?
    public var error: PlayerError?

    public init() {}

    enum CodingKeys: String, CodingKey {
        case status, uid, trackId, position, at, duration, rate, buffering, volume, muted, stream, error
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        status = try c.decodeIfPresent(PlayStatus.self, forKey: .status) ?? .idle
        uid = try c.decodeIfPresent(String.self, forKey: .uid)
        trackId = try c.decodeIfPresent(String.self, forKey: .trackId)
        position = try c.decodeIfPresent(Double.self, forKey: .position) ?? 0
        at = try c.decodeIfPresent(Double.self, forKey: .at) ?? 0
        duration = try c.decodeIfPresent(Double.self, forKey: .duration)
        rate = try c.decodeIfPresent(Double.self, forKey: .rate) ?? 1
        buffering = try c.decodeIfPresent(Bool.self, forKey: .buffering) ?? false
        volume = try c.decodeIfPresent(Double.self, forKey: .volume) ?? 0.7
        muted = try c.decodeIfPresent(Bool.self, forKey: .muted) ?? false
        stream = try c.decodeIfPresent(StreamInfo.self, forKey: .stream)
        error = try c.decodeIfPresent(PlayerError.self, forKey: .error)
    }
}

public struct Continuation: Decodable, Hashable, Sendable {
    public var kind: ContinuationKind = .none
    public var fetching = false

    public init() {}

    enum CodingKeys: String, CodingKey { case kind, fetching }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        kind = try c.decodeIfPresent(ContinuationKind.self, forKey: .kind) ?? .none
        fetching = try c.decodeIfPresent(Bool.self, forKey: .fetching) ?? false
    }
}

/// `QueueSnapshot` (in `player_snapshot`).
public struct QueueSnapshot: Decodable, Sendable {
    public var rev: Int
    public var items: [PlayerTrack]
    public var original: [String]?
    public var index: Int
    public var shuffle: Bool
    public var `repeat`: RepeatMode
    public var continuation: Continuation
}

/// `QueueChanged` (`player:queue`): the order by uid plus what was added or
/// updated since the previous rev.
public struct QueueChanged: Decodable, Sendable {
    public var rev: Int
    public var order: [String]
    public var original: [String]?
    public var index: Int
    public var shuffle: Bool
    public var `repeat`: RepeatMode
    public var continuation: Continuation
    public var added: [PlayerTrack]
    public var updated: [PlayerTrack]
}

/// The player settings the app shows (quality); the rest is kept as is.
public struct PlayerSettings: Decodable, Sendable {
    public var volume: Double
    public var muted: Bool
    /// `high` / `low`.
    public var quality: String
    public var autoplay: Bool

    enum CodingKeys: String, CodingKey { case volume, muted, quality, autoplay }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        volume = try c.decodeIfPresent(Double.self, forKey: .volume) ?? 0.7
        muted = try c.decodeIfPresent(Bool.self, forKey: .muted) ?? false
        quality = try c.decodeIfPresent(String.self, forKey: .quality) ?? "high"
        autoplay = try c.decodeIfPresent(Bool.self, forKey: .autoplay) ?? false
    }

    public init(volume: Double = 0.7, muted: Bool = false, quality: String = "high", autoplay: Bool = false) {
        self.volume = volume
        self.muted = muted
        self.quality = quality
        self.autoplay = autoplay
    }
}

/// `player_snapshot`.
public struct PlayerSnapshot: Decodable, Sendable {
    public var `protocol`: Int
    public var state: PlaybackState
    public var queue: QueueSnapshot
    public var settings: PlayerSettings
    public var sleepEndsAt: Double?
    public var waveActive: Bool
    public var journalSeq: Int

    enum CodingKeys: String, CodingKey { case `protocol`, state, queue, settings, sleepTimer, wave, journalSeq }
    struct Ends: Decodable { var endsAt: Double? }
    struct Wave: Decodable { var active: Bool? }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        `protocol` = try c.decodeIfPresent(Int.self, forKey: .protocol) ?? 0
        state = try c.decode(PlaybackState.self, forKey: .state)
        queue = try c.decode(QueueSnapshot.self, forKey: .queue)
        settings = try c.decodeIfPresent(PlayerSettings.self, forKey: .settings) ?? PlayerSettings()
        sleepEndsAt = try c.decodeIfPresent(Ends.self, forKey: .sleepTimer)?.endsAt
        waveActive = try c.decodeIfPresent(Wave.self, forKey: .wave)?.active ?? false
        journalSeq = try c.decodeIfPresent(Int.self, forKey: .journalSeq) ?? 0
    }
}

/// `player:position`.
public struct PositionEvent: Decodable, Hashable, Sendable {
    public var uid: String?
    public var position: Double
    public var duration: Double?
    public var rate: Double
    public var at: Double
    public var playing: Bool
}

/// Typed player events (`player/forward.rs`).
public enum PlayerEvent: Sendable {
    case state(PlaybackState)
    case position(PositionEvent)
    case queue(QueueChanged)
    case error(PlayerError)
    case settings(PlayerSettings)
    case sleep(endsAt: Double?)
    case wave(active: Bool)
    case queueLow(remaining: Int)
    /// `player:journal`: not needed by the UI beyond a refresh hint.
    case journal

    /// Nil for events that are not the player's, or that do not parse.
    public init?(_ e: CoreEvent) {
        let p = e.payload
        do {
            switch e.name {
            case "player:state": self = .state(try p.decode())
            case "player:position": self = .position(try p.decode())
            case "player:queue": self = .queue(try p.decode())
            case "player:error": self = .error(try p.decode())
            case "player:settings": self = .settings(try p["settings"].decode())
            case "player:sleep": self = .sleep(endsAt: p["endsAt"].double)
            case "player:wave": self = .wave(active: p["active"].bool ?? false)
            case "player:queue-low": self = .queueLow(remaining: p["remaining"].int ?? 0)
            case "player:journal": self = .journal
            default: return nil
            }
        } catch {
            return nil
        }
    }
}
