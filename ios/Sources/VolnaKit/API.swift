import Foundation

/// Typed wrappers over the core's commands: the same names and argument
/// keys as the Flutter app (`app/lib/api`) and the daemon IPC. No logic
/// beyond building arguments and decoding results.
public struct API: Sendable {
    public let core: CoreTransport

    public init(_ core: CoreTransport) {
        self.core = core
    }

    @discardableResult
    func run(_ cmd: String, _ args: JSON = .null) async throws -> JSON {
        try await core.call(cmd, args)
    }

    func get<T: Decodable>(_ cmd: String, _ args: JSON = .null, as _: T.Type = T.self) async throws -> T {
        try await core.call(cmd, args).decode(T.self)
    }

    func rev(_ cmd: String, _ args: JSON = .null) async throws -> Int {
        try await core.call(cmd, args)["rev"].int ?? 0
    }

    static func json<T: Encodable>(_ v: T) -> JSON {
        (try? JSON(encoding: v)) ?? .null
    }

    /// A ytm model as the core takes it (snake_case keys).
    static func snake<T: Encodable>(_ v: T) -> JSON {
        guard let data = try? Wire.snakeEncoder.encode(v) else { return .null }
        return (try? JSON.parse(data)) ?? .null
    }
}

// MARK: - Catalog (`catalog_*`)

public extension API {
    func search(_ query: String, filter: SearchFilter = .all) async throws -> SearchPage {
        try await get("catalog_search", ["query": .string(query), "filter": .string(filter.rawValue)])
    }

    /// The next page of a filtered search; `filter` must be the first page's.
    func searchMore(_ continuation: String, filter: SearchFilter) async throws -> SearchPage {
        try await get("catalog_search_more", ["continuation": .string(continuation), "filter": .string(filter.rawValue)])
    }

    func suggestions(_ input: String) async throws -> SearchSuggestions {
        try await get("catalog_suggestions", ["input": .string(input)])
    }

    func album(_ id: String) async throws -> Album { try await get("catalog_album", ["id": .string(id)]) }
    func artist(_ id: String) async throws -> Artist { try await get("catalog_artist", ["id": .string(id)]) }
    func playlist(_ id: String) async throws -> Playlist { try await get("catalog_playlist", ["id": .string(id)]) }

    func playlistMore(_ continuation: String) async throws -> TrackPage {
        try await get("catalog_playlist_more", ["continuation": .string(continuation)])
    }

    func track(_ videoId: String) async throws -> TrackDetails {
        try await get("catalog_track", ["videoId": .string(videoId)])
    }

    func home(minSections: Int? = nil) async throws -> [Section] {
        try await get("catalog_home", minSections.map { ["minSections": .number(Double($0))] } ?? .null)
    }

    func moods() async throws -> [MoodCategory] { try await get("catalog_moods") }

    func mood(_ params: String) async throws -> [Section] {
        try await get("catalog_mood", ["params": .string(params)])
    }
}

// MARK: - Player (`player_*`)

public enum EnqueueAt: String, Sendable { case next, end }

public extension API {
    func snapshot() async throws -> PlayerSnapshot { try await get("player_snapshot") }

    func play() async throws { try await run("player_play") }
    func pause() async throws { try await run("player_pause") }
    func toggle() async throws { try await run("player_toggle") }
    func stop() async throws { try await run("player_stop") }
    func next() async throws { try await run("player_next") }
    func prev() async throws { try await run("player_prev") }

    /// `position` in seconds; `resume` starts playback when paused.
    func seek(_ position: Double, resume: Bool = false) async throws {
        try await run("player_seek", ["position": .number(position), "resume": .bool(resume)])
    }

    func playUid(_ uid: String, position: Double? = nil) async throws {
        try await run("player_play_uid", ["uid": .string(uid), "position": JSON(position)])
    }

    @discardableResult
    func setQueue(_ tracks: [TrackInput], startIndex: Int = 0, play: Bool = true, position: Double? = nil) async throws -> Int {
        try await rev("player_set_queue", [
            "tracks": API.json(tracks), "startIndex": .number(Double(startIndex)),
            "play": .bool(play), "position": JSON(position),
        ])
    }

    @discardableResult
    func enqueue(_ tracks: [TrackInput], at: EnqueueAt) async throws -> Int {
        try await rev("player_enqueue", ["tracks": API.json(tracks), "at": .string(at.rawValue)])
    }

    @discardableResult
    func remove(uid: String) async throws -> Int { try await rev("player_remove", ["uid": .string(uid)]) }

    @discardableResult
    func move(uid: String, to: Int) async throws -> Int {
        try await rev("player_move", ["uid": .string(uid), "to": .number(Double(to))])
    }

    @discardableResult
    func clearQueue(keepCurrent: Bool = true) async throws -> Int {
        try await rev("player_clear", ["keepCurrent": .bool(keepCurrent)])
    }

    @discardableResult
    func setShuffle(_ on: Bool) async throws -> Int { try await rev("player_set_shuffle", ["on": .bool(on)]) }

    @discardableResult
    func setRepeat(_ mode: RepeatMode) async throws -> Int {
        try await rev("player_set_repeat", ["mode": .string(mode.rawValue)])
    }

    /// Merged one level deep.
    @discardableResult
    func setPlayerSettings(_ patch: JSON) async throws -> PlayerSettings {
        try await get("player_set_settings", ["patch": patch])
    }

    /// nil cancels; returns the end (Unix ms) or nil.
    @discardableResult
    func setSleepTimer(minutes: Double?) async throws -> Double? {
        try await run("player_set_sleep_timer", ["minutes": JSON(minutes)])["endsAt"].double
    }

    func setContinuation(_ kind: ContinuationKind, seeds: [TrackInput] = []) async throws {
        try await run("player_set_continuation", ["kind": .string(kind.rawValue), "seeds": API.json(seeds)])
    }

    @discardableResult
    func waveStart(count: Int = 5) async throws -> Int {
        try await rev("player_wave_start", ["count": .number(Double(count))])
    }

    func waveStop() async throws { try await run("player_wave_stop") }
    func waveDislike() async throws { try await run("player_wave_dislike") }
}

// MARK: - Wave settings (`wave_*`)

public extension API {
    func waveStatus() async throws -> WaveStatus { try await get("wave_status") }

    @discardableResult
    func setWaveSettings(_ s: WaveSettings) async throws -> WaveStatus {
        try await get("wave_set_settings", ["settings": API.json(s)])
    }

    func waveActivities() async throws -> WaveActivities { try await get("wave_activities") }

    /// Empty query: the default grid of popular artists.
    func waveArtistSearch(_ query: String) async throws -> [ArtistCard] {
        try await get("wave_artist_search", ["query": .string(query)])
    }

    /// Seeds the wave with the picked artists.
    @discardableResult
    func waveColdStart(_ artists: [ArtistCard]) async throws -> WaveStatus {
        try await get("wave_cold_start", [
            "artists": .array(artists.map { ["id": .string($0.id), "name": .string($0.name)] }),
        ])
    }
}

// MARK: - Library (`library_*`, `playlist_*`, `history_*`)

public extension API {
    func library(_ kind: LikeKind) async throws -> [Liked] {
        let rows = try await run("library_get", ["kind": .string(kind.rawValue)]).array ?? []
        return rows.compactMap { try? $0.decode(Liked.self) }.filter { $0.item != .unknown }
    }

    /// Liked now.
    @discardableResult
    func like(_ item: Item, on: Bool) async throws -> Bool {
        let kind: LikeKind
        let model: JSON
        switch item {
        case .track(let t): kind = .track; model = API.snake(t)
        case .album(let a): kind = .album; model = API.snake(a)
        case .artist(let a): kind = .artist; model = API.snake(a)
        case .playlist(let p): kind = .playlist; model = API.snake(p)
        case .unknown: return false
        }
        return try await run("library_like", ["kind": .string(kind.rawValue), "item": model, "on": .bool(on)]).bool ?? on
    }

    func isLiked(_ kind: LikeKind, ids: [String]) async throws -> [String: Bool] {
        let o = try await run("library_is_liked", ["kind": .string(kind.rawValue), "ids": JSON(ids)]).object ?? [:]
        return o.compactMapValues(\.bool)
    }

    func playlists() async throws -> [PlaylistInfo] { try await get("playlist_list") }

    func userPlaylist(_ id: String) async throws -> PlaylistDetail {
        try await get("playlist_get", ["id": .string(id)])
    }

    @discardableResult
    func createPlaylist(name: String, description: String? = nil, tracks: [Track] = []) async throws -> PlaylistInfo {
        try await get("playlist_create", [
            "name": .string(name), "description": JSON(description),
            "tracks": .array(tracks.map(API.snake)),
        ])
    }

    @discardableResult
    func renamePlaylist(_ id: String, name: String?, description: String? = nil) async throws -> PlaylistInfo {
        try await get("playlist_rename", ["id": .string(id), "name": JSON(name), "description": JSON(description)])
    }

    @discardableResult
    func deletePlaylist(_ id: String) async throws -> Bool {
        try await run("playlist_delete", ["id": .string(id)]).bool ?? false
    }

    /// Added count.
    @discardableResult
    func addToPlaylist(_ id: String, tracks: [Track]) async throws -> Int {
        try await run("playlist_add", ["id": .string(id), "tracks": .array(tracks.map(API.snake))]).int ?? 0
    }

    /// Removed count.
    @discardableResult
    func removeFromPlaylist(_ id: String, videoIds: [String]) async throws -> Int {
        try await run("playlist_remove", ["id": .string(id), "videoIds": JSON(videoIds)]).int ?? 0
    }

    func movePlaylistTrack(_ id: String, from: Int, to: Int) async throws {
        try await run("playlist_move", ["id": .string(id), "from": .number(Double(from)), "to": .number(Double(to))])
    }

    func history(limit: Int = 100, offset: Int = 0) async throws -> [HistoryEntry] {
        try await get("history_get", ["limit": .number(Double(limit)), "offset": .number(Double(offset))])
    }

    func clearHistory() async throws { try await run("history_clear") }
}

// MARK: - Lyrics, colours

public extension API {
    /// The best lyrics, or nil when no provider has them.
    func lyrics(videoId: String, title: String? = nil, artist: String? = nil, album: String? = nil,
                durationMs: Double? = nil) async throws -> Lyrics?
    {
        let v = try await run("lyrics_get", [
            "videoId": .string(videoId), "title": JSON(title), "artist": JSON(artist),
            "album": JSON(album), "durationMs": JSON(durationMs.map { $0.rounded() }),
        ])
        return v.isNull ? nil : try v.decode(Lyrics.self)
    }

    func palette(_ imageUri: String) async throws -> Palette {
        try await get("color_palette", ["imageUri": .string(imageUri)])
    }
}

// MARK: - Scrobbling (`scrobble_*`)

public extension API {
    func scrobbleStatus() async throws -> ScrobbleStatus { try await get("scrobble_status") }

    /// Only the given fields change.
    @discardableResult
    func scrobbleSet(_ service: ScrobbleService?, enabled: Bool? = nil, loveOnLike: Bool? = nil,
                     token: String? = nil, url: String? = nil, percentage: Int? = nil) async throws -> ScrobbleStatus
    {
        var a: [String: JSON] = [:]
        if let service { a["service"] = .string(service.rawValue) }
        if let enabled { a["enabled"] = .bool(enabled) }
        if let loveOnLike { a["loveOnLike"] = .bool(loveOnLike) }
        if let token { a["token"] = .string(token) }
        if let url { a["url"] = .string(url) }
        if let percentage { a["percentage"] = .number(Double(percentage)) }
        return try await get("scrobble_set", .object(a))
    }

    /// Last.fm / Libre.fm: the page to open in the browser.
    func scrobbleLoginStart(_ s: ScrobbleService) async throws -> URL? {
        try await run("scrobble_login_start", ["service": .string(s.rawValue)])["url"].string.flatMap(URL.init(string:))
    }

    /// After the user allowed access: the username.
    func scrobbleLoginFinish(_ s: ScrobbleService) async throws -> String? {
        try await run("scrobble_login_finish", ["service": .string(s.rawValue)])["username"].string
    }

    @discardableResult
    func scrobbleLogout(_ s: ScrobbleService) async throws -> ScrobbleStatus {
        try await get("scrobble_logout", ["service": .string(s.rawValue)])
    }
}

// MARK: - Yandex (`yandex_*`)

public extension API {
    func yandexStatus() async throws -> YandexStatus { try await get("yandex_status") }

    /// A bare token or the redirect URL with `#access_token=`; checked by the core.
    @discardableResult
    func yandexSetToken(_ input: String) async throws -> YandexStatus {
        try await get("yandex_set_token", ["token": .string(input)])
    }

    /// Runs in the background: follow `yandex_import_progress` / `yandex_import_done`.
    func yandexImportStart(_ o: YandexImportOptions = YandexImportOptions()) async throws {
        try await run("yandex_import_start", API.json(o))
    }

    @discardableResult
    func yandexImportCancel() async throws -> Bool { try await run("yandex_import_cancel").bool ?? false }
}

// MARK: - YouTube session (`youtube_*`)

public struct YouTubeAuth: Decodable, Sendable, Equatable {
    public var signedIn: Bool
    /// The account's display name, when YouTube Music tells it.
    public var account: String?
}

public extension API {
    /// The Google session from the sign-in web view (Netscape cookies.txt);
    /// `""` signs out. The core saves it in `<dataDir>/youtube-cookies.txt`.
    @discardableResult
    func youtubeSetCookies(_ cookies: String) async throws -> YouTubeAuth {
        try await get("youtube_set_cookies", ["cookies": .string(cookies)])
    }

    func youtubeAuthStatus() async throws -> YouTubeAuth { try await get("youtube_auth_status") }
}
