import Foundation

// Library (`crates/core/src/api/library/store.rs`), lyrics
// (`crates/lyrics/src/model.rs`), wave settings (`crates/core/src/wave.rs`),
// scrobblers (`crates/scrobble/src/service.rs`), Yandex import
// (`crates/core/src/api/yandex`), cover colours (`api/color.rs`).

// MARK: - Library

public enum LikeKind: String, Codable, Sendable, CaseIterable {
    case track, album, artist, playlist

    public var label: String {
        switch self {
        case .track: "Треки"
        case .album: "Альбомы"
        case .artist: "Исполнители"
        case .playlist: "Плейлисты"
        }
    }
}

/// `library_get` row.
public struct Liked: Decodable, Sendable, Identifiable {
    public var item: Item
    /// Unix ms.
    public var addedAt: Double
    public var id: String { item.id }
}

public struct PlaylistInfo: Decodable, Sendable, Identifiable, Hashable {
    public var id: String
    public var name: String
    public var description: String
    public var cover: String?
    public var thumbnails: [String]
    public var trackCount: Int
    public var createdAt: Double
    public var updatedAt: Double

    enum CodingKeys: String, CodingKey {
        case id, name, description, cover, thumbnails, trackCount, createdAt, updatedAt
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        cover = try c.decodeIfPresent(String.self, forKey: .cover)
        thumbnails = try c.decodeIfPresent([String].self, forKey: .thumbnails) ?? []
        trackCount = try c.decodeIfPresent(Int.self, forKey: .trackCount) ?? 0
        createdAt = try c.decodeIfPresent(Double.self, forKey: .createdAt) ?? 0
        updatedAt = try c.decodeIfPresent(Double.self, forKey: .updatedAt) ?? 0
    }
}

public struct PlaylistDetail: Decodable, Sendable {
    public struct Entry: Decodable, Sendable, Identifiable {
        public var track: Track
        public var addedAt: Double
        public var id: String { track.videoId }
    }

    public var info: PlaylistInfo
    public var tracks: [Entry]

    enum CodingKeys: String, CodingKey { case tracks }

    public init(from d: Decoder) throws {
        info = try PlaylistInfo(from: d)
        tracks = try d.container(keyedBy: CodingKeys.self).decodeIfPresent([Entry].self, forKey: .tracks) ?? []
    }
}

public struct HistoryEntry: Decodable, Sendable, Identifiable {
    public var track: Track
    public var playedAt: Double
    public var id: String { "\(track.videoId)@\(Int(playedAt))" }
}

// MARK: - Lyrics

public struct Lyrics: Decodable, Sendable, Equatable {
    public enum Synced: String, Decodable, Sendable {
        case none, line, word

        public init(from d: Decoder) throws {
            self = Synced(rawValue: try d.singleValueContainer().decode(String.self)) ?? .none
        }
    }

    public struct Word: Decodable, Sendable, Equatable {
        public var startMs: Double
        public var endMs: Double
        public var text: String
    }

    public struct Line: Decodable, Sendable, Equatable {
        public var startMs: Double
        public var endMs: Double
        public var text: String
        public var words: [Word]?
    }

    public var source: String
    public var synced: Synced
    public var lines: [Line]
    public var plain: String?
    public var songwriters: String?

    /// The line playing at `positionMs` (the last one that started), or nil
    /// before the first or for unsynced lyrics.
    public func lineIndex(at positionMs: Double) -> Int? {
        guard synced != .none, !lines.isEmpty else { return nil }
        var lo = 0, hi = lines.count - 1, found: Int?
        while lo <= hi {
            let mid = (lo + hi) / 2
            if lines[mid].startMs <= positionMs {
                found = mid
                lo = mid + 1
            } else {
                hi = mid - 1
            }
        }
        return found
    }
}

// MARK: - Wave settings

public struct WaveActivity: Codable, Sendable, Hashable, Identifiable {
    /// YouTube Music title, the stable key.
    public var title: String
    public var params: String
    /// Russian label (only in `wave_activities`).
    public var label: String?
    public var id: String { title }

    public init(title: String, params: String, label: String? = nil) {
        self.title = title
        self.params = params
        self.label = label
    }

    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: CodingKeys.self)
        try c.encode(title, forKey: .title)
        try c.encode(params, forKey: .params)
    }
}

public struct WaveActivities: Decodable, Sendable {
    public var activities: [WaveActivity]
    public var genres: [WaveActivity]
}

public struct WaveSettings: Codable, Sendable, Equatable {
    /// `default`, `favorite`, `unfamiliar`, `popular`.
    public var character: String = "default"
    /// `energetic`, `happy`, `calm`, `sad`, or nil.
    public var mood: String?
    public var activity: WaveActivity?
    /// `any`, `russian`, `uzbek`, `english`.
    public var language: String = "any"

    public init() {}

    enum CodingKeys: String, CodingKey { case character, mood, activity, language }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        character = try c.decodeIfPresent(String.self, forKey: .character) ?? "default"
        mood = try c.decodeIfPresent(String.self, forKey: .mood)
        activity = try c.decodeIfPresent(WaveActivity.self, forKey: .activity)
        language = try c.decodeIfPresent(String.self, forKey: .language) ?? "any"
    }

    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: CodingKeys.self)
        try c.encode(character, forKey: .character)
        try c.encode(mood, forKey: .mood)
        try c.encode(activity, forKey: .activity)
        try c.encode(language, forKey: .language)
    }

    public static let characters: [(String, String)] = [
        ("default", "Любой"), ("favorite", "Любимое"), ("unfamiliar", "Незнакомое"), ("popular", "Популярное"),
    ]
    public static let moods: [(String, String)] = [
        ("energetic", "Бодрое"), ("happy", "Весёлое"), ("calm", "Спокойное"), ("sad", "Грустное"),
    ]
    public static let languages: [(String, String)] = [
        ("any", "Любой"), ("russian", "Русский"), ("uzbek", "Узбекский"), ("english", "Английский"),
    ]
}

public struct WaveStatus: Decodable, Sendable {
    /// No taste yet: the cold-start picker comes first.
    public var cold: Bool
    public var active: Bool
    public var settings: WaveSettings

    enum CodingKeys: String, CodingKey { case cold, active, settings }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        cold = try c.decodeIfPresent(Bool.self, forKey: .cold) ?? false
        active = try c.decodeIfPresent(Bool.self, forKey: .active) ?? false
        settings = try c.decodeIfPresent(WaveSettings.self, forKey: .settings) ?? WaveSettings()
    }
}

/// An artist tile of the cold-start picker; `picture` is a `volna://img` URL.
public struct ArtistCard: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    public var name: String
    public var picture: String?

    public init(id: String, name: String, picture: String? = nil) {
        self.id = id
        self.name = name
        self.picture = picture
    }
}

// MARK: - Scrobbling

public enum ScrobbleService: String, Codable, Sendable, CaseIterable, Identifiable {
    case lastfm, librefm, listenbrainz, maloja
    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .lastfm: "Last.fm"
        case .librefm: "Libre.fm"
        case .listenbrainz: "ListenBrainz"
        case .maloja: "Maloja"
        }
    }
}

public struct ScrobbleStatus: Decodable, Sendable {
    public struct Queue: Decodable, Sendable {
        public var pending: Int?
        /// Unix seconds.
        public var nextRetryAt: Double?
        public var lastError: String?
    }

    public struct Service: Decodable, Sendable, Identifiable {
        public var service: ScrobbleService
        public var enabled: Bool?
        /// Enabled and has credentials: plays are sent.
        public var active: Bool?
        public var canLove: Bool?
        public var loveOnLike: Bool?
        /// Last.fm and Libre.fm log in through the browser.
        public var usesLogin: Bool?
        public var username: String?
        public var loginPending: Bool?
        public var hasToken: Bool?
        public var url: String?
        public var queue: Queue?
        public var id: String { service.rawValue }
    }

    /// Share of the track to play before it is scrobbled (or 4 minutes).
    public var percentage: Int?
    public var services: [Service]

    public func of(_ s: ScrobbleService) -> Service? { services.first { $0.service == s } }
}

// MARK: - Yandex

/// The Yandex OAuth app of the core (`client.rs::CLIENT_ID`); its authorize
/// page redirects to a URL with `#access_token=`.
public enum Yandex {
    public static let clientId = "23cabbbdc6cd418abb4b39c32c41195d"

    public static func authorizeURL(host: String = "oauth.yandex.ru") -> URL {
        URL(string: "https://\(host)/authorize?response_type=token&client_id=\(clientId)")!
    }

    /// The token in a redirect URL (`…#access_token=…&…`), if any.
    public static func token(fromRedirect url: String) -> String? {
        guard let r = url.range(of: "access_token=") else { return nil }
        let rest = url[r.upperBound...]
        let token = rest.prefix { $0 != "&" && $0 != "#" }
        return token.isEmpty ? nil : String(token)
    }
}

public struct YandexStatus: Decodable, Sendable {
    public var signedIn: Bool?
    public var login: String?
    public var importing: Bool?
}

public struct YandexImportOptions: Encodable, Sendable {
    public var likes = true
    public var playlists = true
    public var albums = true
    public var artists = true
    /// No browser session on iOS: the token comes from the login page.
    public var browser = false
    public init() {}
}

/// `yandex_import_progress`.
public struct YandexProgress: Decodable, Sendable {
    public var stage: String?
    public var detail: String?
    public var done: Int?
    public var total: Int?
    public var matched: Int?
    public var current: String?

    public var fraction: Double? {
        guard let total, total > 0 else { return nil }
        return min(1, Double(done ?? 0) / Double(total))
    }
}

/// `yandex_import_done`.
public struct YandexDone: Decodable, Sendable {
    public struct Summary: Decodable, Sendable {
        public var likedTracks: Int?
        public var playlists: Int?
        public var albums: Int?
        public var artists: Int?
        public var matched: Int?
        public var total: Int?
    }

    public var ok: Bool?
    public var cancelled: Bool?
    /// No usable token: log in, `yandex_set_token`, start again.
    public var needLogin: Bool?
    public var error: String?
    public var summary: Summary?
}

// MARK: - Colours

/// `color_palette`: hex colours of a cover.
public struct Palette: Decodable, Sendable, Equatable {
    public var dominant: String
    public var accent: String
    public var onDominant: String
    public var onAccent: String
    public var dark: Bool

    enum CodingKeys: String, CodingKey { case dominant, accent, onDominant, onAccent, dark }

    /// `#rrggbb` → (r, g, b) in 0…1.
    public static func rgb(_ hex: String) -> (Double, Double, Double)? {
        let h = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
        guard h.count == 6, let v = UInt32(h, radix: 16) else { return nil }
        return (Double((v >> 16) & 0xff) / 255, Double((v >> 8) & 0xff) / 255, Double(v & 0xff) / 255)
    }
}

/// `volna://img/?u=<url>&s=<px>`: a cover through the core (cache + the
/// core's network), never straight from the web view or URLSession.
public func imageURI(_ url: String, size: Int? = nil) -> String {
    if url.hasPrefix("volna://") { return url }
    var allowed = CharacterSet.alphanumerics
    allowed.insert(charactersIn: "-._~")
    let enc = url.addingPercentEncoding(withAllowedCharacters: allowed) ?? url
    return size.map { "volna://img/?u=\(enc)&s=\($0)" } ?? "volna://img/?u=\(enc)"
}
