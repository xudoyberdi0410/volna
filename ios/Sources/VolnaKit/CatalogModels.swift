import Foundation

// The serde shapes of `crates/ytm/src/models.rs` (snake_case keys on the
// wire, mapped by `Wire.decoder`). Optional collections default to empty,
// like `#[serde(default)]` on the Rust side.

public struct Thumbnail: Codable, Hashable, Sendable {
    public var url: String
    public var width: Int
    public var height: Int

    public init(url: String, width: Int = 0, height: Int = 0) {
        self.url = url
        self.width = width
        self.height = height
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        url = try c.decode(String.self, forKey: .url)
        width = try c.decodeIfPresent(Int.self, forKey: .width) ?? 0
        height = try c.decodeIfPresent(Int.self, forKey: .height) ?? 0
    }
}

public extension Array where Element == Thumbnail {
    /// The largest thumbnail.
    var best: Thumbnail? { self.max { $0.width * $0.height < $1.width * $1.height } }
}

public struct ArtistRef: Codable, Hashable, Sendable {
    /// Channel id (`UC…`); nil for unlinked names.
    public var id: String?
    public var name: String

    public init(id: String?, name: String) {
        self.id = id
        self.name = name
    }
}

public struct AlbumRef: Codable, Hashable, Sendable {
    public var id: String?
    public var name: String

    public init(id: String?, name: String) {
        self.id = id
        self.name = name
    }
}

public enum TrackKind: String, Codable, Sendable {
    case song, officialVideo = "official_video", video

    public init(from d: Decoder) throws {
        self = TrackKind(rawValue: try d.singleValueContainer().decode(String.self)) ?? .song
    }
}

public struct Track: Codable, Hashable, Sendable, Identifiable {
    public var videoId: String
    public var title: String
    public var artists: [ArtistRef]
    public var album: AlbumRef?
    public var durationS: Int?
    public var thumbnails: [Thumbnail]
    public var explicit: Bool
    public var kind: TrackKind
    public var year: Int?
    public var views: String?
    public var playable: Bool

    public var id: String { videoId }

    public init(videoId: String, title: String, artists: [ArtistRef] = [], album: AlbumRef? = nil,
                durationS: Int? = nil, thumbnails: [Thumbnail] = [], explicit: Bool = false,
                kind: TrackKind = .song, year: Int? = nil, views: String? = nil, playable: Bool = true)
    {
        self.videoId = videoId
        self.title = title
        self.artists = artists
        self.album = album
        self.durationS = durationS
        self.thumbnails = thumbnails
        self.explicit = explicit
        self.kind = kind
        self.year = year
        self.views = views
        self.playable = playable
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        videoId = try c.decode(String.self, forKey: .videoId)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        artists = try c.decodeIfPresent([ArtistRef].self, forKey: .artists) ?? []
        album = try c.decodeIfPresent(AlbumRef.self, forKey: .album)
        durationS = try c.decodeIfPresent(Int.self, forKey: .durationS)
        thumbnails = try c.decodeIfPresent([Thumbnail].self, forKey: .thumbnails) ?? []
        explicit = try c.decodeIfPresent(Bool.self, forKey: .explicit) ?? false
        kind = try c.decodeIfPresent(TrackKind.self, forKey: .kind) ?? .song
        year = try c.decodeIfPresent(Int.self, forKey: .year)
        views = try c.decodeIfPresent(String.self, forKey: .views)
        playable = try c.decodeIfPresent(Bool.self, forKey: .playable) ?? true
    }

    /// "Artist, Artist".
    public var artistNames: String { artists.map(\.name).filter { !$0.isEmpty }.joined(separator: ", ") }
}

public enum AlbumKind: String, Codable, Sendable {
    case album, single, ep

    public init(from d: Decoder) throws {
        self = AlbumKind(rawValue: try d.singleValueContainer().decode(String.self)) ?? .album
    }

    public var label: String {
        switch self {
        case .album: "Альбом"
        case .single: "Сингл"
        case .ep: "EP"
        }
    }
}

public struct AlbumSummary: Codable, Hashable, Sendable, Identifiable {
    public var browseId: String
    public var playlistId: String?
    public var title: String
    public var kind: AlbumKind
    public var artists: [ArtistRef]
    public var year: Int?
    public var thumbnails: [Thumbnail]
    public var explicit: Bool

    public var id: String { browseId }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        browseId = try c.decode(String.self, forKey: .browseId)
        playlistId = try c.decodeIfPresent(String.self, forKey: .playlistId)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        kind = try c.decodeIfPresent(AlbumKind.self, forKey: .kind) ?? .album
        artists = try c.decodeIfPresent([ArtistRef].self, forKey: .artists) ?? []
        year = try c.decodeIfPresent(Int.self, forKey: .year)
        thumbnails = try c.decodeIfPresent([Thumbnail].self, forKey: .thumbnails) ?? []
        explicit = try c.decodeIfPresent(Bool.self, forKey: .explicit) ?? false
    }
}

public struct ArtistSummary: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var thumbnails: [Thumbnail]
    public var audience: String?

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        thumbnails = try c.decodeIfPresent([Thumbnail].self, forKey: .thumbnails) ?? []
        audience = try c.decodeIfPresent(String.self, forKey: .audience)
    }
}

public struct PlaylistSummary: Codable, Hashable, Sendable, Identifiable {
    /// Without the `VL` prefix.
    public var id: String
    public var title: String
    public var author: ArtistRef?
    public var thumbnails: [Thumbnail]
    public var trackCount: Int?
    public var subtitle: String?

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        author = try c.decodeIfPresent(ArtistRef.self, forKey: .author)
        thumbnails = try c.decodeIfPresent([Thumbnail].self, forKey: .thumbnails) ?? []
        trackCount = try c.decodeIfPresent(Int.self, forKey: .trackCount)
        subtitle = try c.decodeIfPresent(String.self, forKey: .subtitle)
    }
}

/// `Item`: internally tagged by `type`. Unknown types decode as `.unknown`
/// and are dropped by the lists.
public enum Item: Codable, Hashable, Sendable, Identifiable {
    case track(Track)
    case album(AlbumSummary)
    case artist(ArtistSummary)
    case playlist(PlaylistSummary)
    case unknown

    private enum TypeKey: String, CodingKey { case type }

    public init(from d: Decoder) throws {
        let type = try d.container(keyedBy: TypeKey.self).decodeIfPresent(String.self, forKey: .type)
        switch type {
        case "track": self = .track(try Track(from: d))
        case "album": self = .album(try AlbumSummary(from: d))
        case "artist": self = .artist(try ArtistSummary(from: d))
        case "playlist": self = .playlist(try PlaylistSummary(from: d))
        default: self = .unknown
        }
    }

    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: TypeKey.self)
        switch self {
        case .track(let t): try c.encode("track", forKey: .type); try t.encode(to: e)
        case .album(let a): try c.encode("album", forKey: .type); try a.encode(to: e)
        case .artist(let a): try c.encode("artist", forKey: .type); try a.encode(to: e)
        case .playlist(let p): try c.encode("playlist", forKey: .type); try p.encode(to: e)
        case .unknown: try c.encode("unknown", forKey: .type)
        }
    }

    public var id: String {
        switch self {
        case .track(let t): "t:" + t.videoId
        case .album(let a): "a:" + a.browseId
        case .artist(let a): "r:" + a.id
        case .playlist(let p): "p:" + p.id
        case .unknown: "?"
        }
    }

    public var title: String {
        switch self {
        case .track(let t): t.title
        case .album(let a): a.title
        case .artist(let a): a.name
        case .playlist(let p): p.title
        case .unknown: ""
        }
    }

    public var thumbnails: [Thumbnail] {
        switch self {
        case .track(let t): t.thumbnails
        case .album(let a): a.thumbnails
        case .artist(let a): a.thumbnails
        case .playlist(let p): p.thumbnails
        case .unknown: []
        }
    }

    /// One line under the title, in Russian.
    public var subtitle: String {
        switch self {
        case .track(let t): ([t.kind == .song ? "Трек" : "Видео", t.artistNames].filter { !$0.isEmpty }).joined(separator: " · ")
        case .album(let a): ([a.kind.label, a.artists.map(\.name).joined(separator: ", "), a.year.map(String.init) ?? ""]
            .filter { !$0.isEmpty }).joined(separator: " · ")
        case .artist(let a): (["Исполнитель", a.audience ?? ""].filter { !$0.isEmpty }).joined(separator: " · ")
        case .playlist(let p): (["Плейлист", p.author?.name ?? p.subtitle ?? ""].filter { !$0.isEmpty }).joined(separator: " · ")
        case .unknown: ""
        }
    }
}

/// Decodes a list leniently: entries of unknown `type` are dropped.
struct ItemList: Decodable {
    var items: [Item]
    init(from d: Decoder) throws {
        var c = try d.unkeyedContainer()
        var out: [Item] = []
        while !c.isAtEnd {
            let item = try c.decode(Item.self)
            if item != .unknown { out.append(item) }
        }
        items = out
    }
}

public struct Section: Decodable, Sendable, Identifiable {
    public var title: String
    public var strapline: String?
    public var items: [Item]
    public var id: String { title }

    enum CodingKeys: String, CodingKey { case title, strapline, items }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        strapline = try c.decodeIfPresent(String.self, forKey: .strapline)
        items = try c.decodeIfPresent(ItemList.self, forKey: .items)?.items ?? []
    }
}

public enum SearchFilter: String, CaseIterable, Sendable {
    case all, songs, videos, albums, artists, playlists

    public var label: String {
        switch self {
        case .all: "Всё"
        case .songs: "Треки"
        case .videos: "Видео"
        case .albums: "Альбомы"
        case .artists: "Исполнители"
        case .playlists: "Плейлисты"
        }
    }
}

public struct SearchPage: Decodable, Sendable {
    public var topResult: Item?
    public var items: [Item]
    public var continuation: String?

    enum CodingKeys: String, CodingKey { case topResult, items, continuation }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        let top = try c.decodeIfPresent(Item.self, forKey: .topResult)
        topResult = top == .unknown ? nil : top
        items = try c.decodeIfPresent(ItemList.self, forKey: .items)?.items ?? []
        continuation = try c.decodeIfPresent(String.self, forKey: .continuation)
    }
}

public struct SearchSuggestions: Decodable, Sendable {
    public var queries: [String]
    public var items: [Item]

    enum CodingKeys: String, CodingKey { case queries, items }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        queries = try c.decodeIfPresent([String].self, forKey: .queries) ?? []
        items = try c.decodeIfPresent(ItemList.self, forKey: .items)?.items ?? []
    }
}

public struct Album: Decodable, Sendable {
    public var browseId: String
    public var playlistId: String?
    public var title: String
    public var kind: AlbumKind
    public var artists: [ArtistRef]
    public var year: Int?
    public var description: String?
    public var thumbnails: [Thumbnail]
    public var explicit: Bool
    public var trackCount: Int?
    public var durationS: Int
    public var tracks: [Track]
    public var related: [AlbumSummary]

    enum CodingKeys: String, CodingKey {
        case browseId, playlistId, title, kind, artists, year, description, thumbnails, explicit,
             trackCount, durationS, tracks, related
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        browseId = try c.decode(String.self, forKey: .browseId)
        playlistId = try c.decodeIfPresent(String.self, forKey: .playlistId)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        kind = try c.decodeIfPresent(AlbumKind.self, forKey: .kind) ?? .album
        artists = try c.decodeIfPresent([ArtistRef].self, forKey: .artists) ?? []
        year = try c.decodeIfPresent(Int.self, forKey: .year)
        description = try c.decodeIfPresent(String.self, forKey: .description)
        thumbnails = try c.decodeIfPresent([Thumbnail].self, forKey: .thumbnails) ?? []
        explicit = try c.decodeIfPresent(Bool.self, forKey: .explicit) ?? false
        trackCount = try c.decodeIfPresent(Int.self, forKey: .trackCount)
        durationS = try c.decodeIfPresent(Int.self, forKey: .durationS) ?? 0
        tracks = try c.decodeIfPresent([Track].self, forKey: .tracks) ?? []
        related = try c.decodeIfPresent([AlbumSummary].self, forKey: .related) ?? []
    }
}

public struct Artist: Decodable, Sendable {
    public var id: String
    public var name: String
    public var description: String?
    public var thumbnails: [Thumbnail]
    public var subscribers: String?
    public var shufflePlaylistId: String?
    public var radioPlaylistId: String?
    public var topSongs: [Track]
    public var topSongsPlaylistId: String?
    public var albums: [AlbumSummary]
    public var singles: [AlbumSummary]
    public var videos: [Track]
    public var playlists: [PlaylistSummary]
    public var related: [ArtistSummary]

    enum CodingKeys: String, CodingKey {
        case id, name, description, thumbnails, subscribers, shufflePlaylistId, radioPlaylistId,
             topSongs, topSongsPlaylistId, albums, singles, videos, playlists, related
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        description = try c.decodeIfPresent(String.self, forKey: .description)
        thumbnails = try c.decodeIfPresent([Thumbnail].self, forKey: .thumbnails) ?? []
        subscribers = try c.decodeIfPresent(String.self, forKey: .subscribers)
        shufflePlaylistId = try c.decodeIfPresent(String.self, forKey: .shufflePlaylistId)
        radioPlaylistId = try c.decodeIfPresent(String.self, forKey: .radioPlaylistId)
        topSongs = try c.decodeIfPresent([Track].self, forKey: .topSongs) ?? []
        topSongsPlaylistId = try c.decodeIfPresent(String.self, forKey: .topSongsPlaylistId)
        albums = try c.decodeIfPresent([AlbumSummary].self, forKey: .albums) ?? []
        singles = try c.decodeIfPresent([AlbumSummary].self, forKey: .singles) ?? []
        videos = try c.decodeIfPresent([Track].self, forKey: .videos) ?? []
        playlists = try c.decodeIfPresent([PlaylistSummary].self, forKey: .playlists) ?? []
        related = try c.decodeIfPresent([ArtistSummary].self, forKey: .related) ?? []
    }
}

public struct Playlist: Decodable, Sendable {
    public var id: String
    public var title: String
    public var author: ArtistRef?
    public var description: String?
    public var thumbnails: [Thumbnail]
    public var year: Int?
    public var trackCount: Int?
    public var tracks: [Track]
    public var continuation: String?

    enum CodingKeys: String, CodingKey {
        case id, title, author, description, thumbnails, year, trackCount, tracks, continuation
    }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        author = try c.decodeIfPresent(ArtistRef.self, forKey: .author)
        description = try c.decodeIfPresent(String.self, forKey: .description)
        thumbnails = try c.decodeIfPresent([Thumbnail].self, forKey: .thumbnails) ?? []
        year = try c.decodeIfPresent(Int.self, forKey: .year)
        trackCount = try c.decodeIfPresent(Int.self, forKey: .trackCount)
        tracks = try c.decodeIfPresent([Track].self, forKey: .tracks) ?? []
        continuation = try c.decodeIfPresent(String.self, forKey: .continuation)
    }
}

public struct TrackPage: Decodable, Sendable {
    public var tracks: [Track]
    public var continuation: String?

    enum CodingKeys: String, CodingKey { case tracks, continuation }

    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: CodingKeys.self)
        tracks = try c.decodeIfPresent([Track].self, forKey: .tracks) ?? []
        continuation = try c.decodeIfPresent(String.self, forKey: .continuation)
    }
}

public struct TrackDetails: Decodable, Sendable {
    public var track: Track
    public var lyricsBrowseId: String?
    public var relatedBrowseId: String?
}

public struct MoodCategory: Decodable, Sendable, Identifiable {
    public struct Mood: Decodable, Sendable, Identifiable {
        public var title: String
        public var params: String
        /// ARGB.
        public var color: UInt32?
        public var id: String { params }
    }

    public var title: String
    public var moods: [Mood]
    public var id: String { title }
}
