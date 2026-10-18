import Foundation
import XCTest
@testable import VolnaKit

/// The Swift bridge against the real core (`Libs/linux-x86_64/libvolna_core.a`,
/// built by `scripts/build-core.sh linux`): offline commands, events, images,
/// the player and JS hooks, shutdown.
final class CoreTests: XCTestCase {
    var dir: URL!
    var core: Core!
    var api: API!

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("volnakit-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        core = try Core(config: CoreConfig(dataDir: dir.path, audioServer: true))
        api = API(core)
    }

    override func tearDownWithError() throws {
        core.shutdown()
        try? FileManager.default.removeItem(at: dir)
    }

    func testBadConfigFailsWithTheReason() {
        XCTAssertThrowsError(try Core(config: CoreConfig(dataDir: dir.path, devProxy: "ftp://nope"))) { e in
            let err = e as! CoreError
            XCTAssertEqual(err.kind, "init")
            XCTAssertTrue(err.message.contains("devProxy"), err.message)
        }
    }

    func testSnapshotDecodes() async throws {
        let s = try await api.snapshot()
        XCTAssertEqual(s.protocol, 1)
        XCTAssertEqual(s.state.status, .idle)
        XCTAssertEqual(s.queue.index, -1)
        XCTAssertEqual(s.settings.quality, "high")
        var m = PlayerModel()
        m.apply(s)
        XCTAssertNil(m.current)
        XCTAssertEqual(m.rev, s.queue.rev)
    }

    func testErrorsCarryTheirKind() async throws {
        do {
            _ = try await core.call("no_such_command", [:])
            XCTFail("no error")
        } catch let e as CoreError {
            XCTAssertTrue(e.message.contains("unknown command"), e.description)
        }
        do {
            _ = try await core.call("net_apply", ["config": ["mode": "url", "value": "socks5h://127.0.0.1:9"]])
            XCTFail("no error")
        } catch let e as CoreError {
            XCTAssertEqual(e.kind, "unsupported")
        }
        do {
            _ = try await api.userPlaylist("missing")
            XCTFail("no error")
        } catch let e as CoreError {
            XCTAssertEqual(e.kind, "not_found", e.description)
        }
    }

    func testPlaylistsRoundTrip() async throws {
        let a = Track(videoId: "dQw4w9WgXcQ", title: "Never Gonna Give You Up",
                      artists: [ArtistRef(id: "UCuAXFkgsw1L7xaCfnd5JJOw", name: "Rick Astley")],
                      album: AlbumRef(id: "MPREb_x", name: "Whenever You Need Somebody"), durationS: 213,
                      thumbnails: [Thumbnail(url: "https://lh3.googleusercontent.com/x=w120-h120", width: 120, height: 120)])
        let b = Track(videoId: "yPYZpwSpKmA", title: "Together Forever", artists: [ArtistRef(id: nil, name: "Rick Astley")])
        let created = try await api.createPlaylist(name: "Тест", description: "описание", tracks: [a])
        XCTAssertEqual(created.name, "Тест")
        XCTAssertEqual(created.trackCount, 1)
        let v1 = try await api.addToPlaylist(created.id, tracks: [b])
        XCTAssertEqual(v1, 1)
        try await api.movePlaylistTrack(created.id, from: 1, to: 0)
        var detail = try await api.userPlaylist(created.id)
        XCTAssertEqual(detail.tracks.map(\.track.videoId), [b.videoId, a.videoId])
        XCTAssertEqual(detail.tracks[1].track.artists.first?.id, "UCuAXFkgsw1L7xaCfnd5JJOw")
        XCTAssertEqual(detail.tracks[1].track.durationS, 213)
        XCTAssertEqual(detail.tracks[1].track.album?.name, "Whenever You Need Somebody")
        let renamed = try await api.renamePlaylist(created.id, name: "Новое имя")
        XCTAssertEqual(renamed.name, "Новое имя")
        let v2 = try await api.removeFromPlaylist(created.id, videoIds: [b.videoId])
        XCTAssertEqual(v2, 1)
        detail = try await api.userPlaylist(created.id)
        XCTAssertEqual(detail.info.trackCount, 1)
        let v3 = try await api.playlists().map(\.id)
        XCTAssertEqual(v3, [created.id])
        let v4 = try await api.deletePlaylist(created.id)
        XCTAssertTrue(v4)
        let v5 = try await api.playlists().count
        XCTAssertEqual(v5, 0)
    }

    func testLikesAndTheirEvent() async throws {
        let events = core.events()
        let t = Track(videoId: "dQw4w9WgXcQ", title: "Never Gonna Give You Up",
                      artists: [ArtistRef(id: nil, name: "Rick Astley")], durationS: 213)
        let v6 = try await api.like(.track(t), on: true)
        XCTAssertTrue(v6)
        let v7 = try await api.isLiked(.track, ids: [t.videoId, "xxxxxxxxxxx"])
        XCTAssertEqual(v7,
                       [t.videoId: true, "xxxxxxxxxxx": false])
        let liked = try await api.library(.track)
        XCTAssertEqual(liked.count, 1)
        guard case .track(let back) = liked[0].item else { return XCTFail("\(liked[0].item)") }
        XCTAssertEqual(back.title, t.title)
        XCTAssertGreaterThan(liked[0].addedAt, 0)
        let name = try await firstEvent(events) { $0.name == "library_changed" }
        XCTAssertEqual(name.payload["kind"].string, "track")
        let v8 = try await api.like(.track(t), on: false)
        XCTAssertFalse(v8)
        let v9 = try await api.library(.track).count
        XCTAssertEqual(v9, 0)
    }

    func testOfflineSettingsCommandsDecode() async throws {
        let wave = try await api.waveStatus()
        XCTAssertTrue(wave.cold)
        var s = wave.settings
        s.character = "favorite"
        s.mood = "calm"
        let saved = try await api.setWaveSettings(s)
        XCTAssertEqual(saved.settings.character, "favorite")
        XCTAssertEqual(saved.settings.mood, "calm")
        let scrobble = try await api.scrobbleStatus()
        XCTAssertEqual(Set(scrobble.services.map(\.service)), Set(ScrobbleService.allCases))
        let changed = try await api.scrobbleSet(.listenbrainz, enabled: true, token: "t0ken")
        XCTAssertEqual(changed.of(.listenbrainz)?.enabled, true)
        XCTAssertEqual(changed.of(.listenbrainz)?.hasToken, true)
        let yandex = try await api.yandexStatus()
        XCTAssertEqual(yandex.signedIn, false)
        let v10 = try await api.history().count
        XCTAssertEqual(v10, 0)
        let quality = try await api.setPlayerSettings(["quality": "low"])
        XCTAssertEqual(quality.quality, "low")
    }

    func testYouTubeSessionCommands() async throws {
        let signedOut = try await api.youtubeAuthStatus()
        XCTAssertFalse(signedOut.signedIn)
        // Not a session (no LOGIN_INFO): saved, but not signed in.
        let partial = "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tPREF\tf6=1\n"
        let s = try await api.youtubeSetCookies(partial)
        XCTAssertFalse(s.signedIn)
        let file = dir.appendingPathComponent("youtube-cookies.txt")
        XCTAssertEqual(try String(contentsOf: file, encoding: .utf8), partial)
        let out = try await api.youtubeSetCookies("")
        XCTAssertFalse(out.signedIn)
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
    }

    func testImagesThroughProto() async throws {
        let r = await core.proto("volna://img/?s=80")
        XCTAssertEqual(r.status, 400)
        XCTAssertNotNil(r.header("Content-Type"))
        XCTAssertEqual(try JSON.parse(r.body)["status"].int, 400)
    }

    func testQueueDrivesThePlatformPlayer() async throws {
        let events = core.events()
        let commands = Recorder<PlayerCommand>()
        let core = self.core!
        // The app's AVPlayer side, minus AVPlayer: the bridge over a fake engine.
        final class Engine: MediaEngine {
            var loads: [URL] = []
            func load(url: URL, startSeconds: Double, token: Int) { loads.append(url) }
            func play() {}
            func pause() {}
            func seek(seconds: Double) {}
            func stop() {}
            func setVolume(_ v: Double) {}
        }
        let engine = Engine()
        let bridge = PlatformPlayerBridge(engine: engine) { core.report($0) }
        let lock = NSLock()
        core.setPlayerHandler { j in
            guard let cmd = PlayerCommand(j) else { return }
            commands.add(cmd)
            lock.withLock { bridge.handle(cmd) }
        }
        let t = TrackInput(id: "dQw4w9WgXcQ", title: "A", artists: [PArtist(id: nil, name: "B")], duration: 213)
        let rev = try await api.setQueue([t], play: true)
        XCTAssertGreaterThan(rev, 0)
        var model = PlayerModel()
        model.apply(try await api.snapshot())
        XCTAssertEqual(model.queue.map(\.trackId), ["dQw4w9WgXcQ"])
        XCTAssertEqual(model.current?.title, "A")

        // The core asks for the loopback URL, AAC only (the iOS init option).
        let deadline = Date().addingTimeInterval(20)
        while engine.loads.isEmpty, Date() < deadline { try await Task.sleep(nanoseconds: 50_000_000) }
        let url = try XCTUnwrap(engine.loads.first)
        XCTAssertEqual(url.host, "127.0.0.1")
        XCTAssertTrue(url.path.hasSuffix("/dQw4w9WgXcQ"), url.path)
        XCTAssertEqual(url.query, "codec=aac")

        // The engine fails to open it: the core hears it as a player error.
        lock.withLock { bridge.engineFailed(token: bridge.token, message: "HTTP 500") }
        let err = try await firstEvent(events) { e in
            if let pe = PlayerEvent(e) { model.apply(pe) }
            return e.name == "player:error"
        }
        guard case .error(let pe)? = PlayerEvent(err) else { return XCTFail("\(err)") }
        XCTAssertEqual(pe.trackId, "dQw4w9WgXcQ")
        XCTAssertFalse(core.jsResult(42, .success("1")))
    }

    func testShutdownFailsLaterCalls() async throws {
        core.shutdown()
        do {
            _ = try await api.snapshot()
            XCTFail("no error")
        } catch let e as CoreError {
            XCTAssertEqual(e.kind, "closed")
        }
        core.shutdown() // idempotent
    }

    // MARK: - Live (network through VOLNA_TEST_PROXY)

    /// `VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 swift test --filter Live`
    func testLiveCatalogThroughDevProxy() async throws {
        guard let proxy = ProcessInfo.processInfo.environment["VOLNA_TEST_PROXY"] else {
            throw XCTSkip("set VOLNA_TEST_PROXY to run the live catalog test")
        }
        core.shutdown()
        core = try Core(config: CoreConfig(dataDir: dir.path, devProxy: proxy))
        api = API(core)

        do {
            let page = try await api.search("Rick Astley Never Gonna Give You Up")
            XCTAssertFalse(page.items.isEmpty)
            let songs = try await api.search("Rick Astley", filter: .songs)
            let song = try XCTUnwrap(songs.items.compactMap { if case .track(let t) = $0 { t } else { nil } }.first)
            XCTAssertEqual(song.videoId.count, 11)
            XCTAssertFalse(song.thumbnails.isEmpty)
            if let c = songs.continuation {
                let more = try await api.searchMore(c, filter: .songs)
                XCTAssertFalse(more.items.isEmpty)
            }
            let sugg = try await api.suggestions("rick ast")
            XCTAssertFalse(sugg.queries.isEmpty)

            let artistId = try XCTUnwrap(song.artists.compactMap(\.id).first)
            let artist = try await api.artist(artistId)
            XCTAssertFalse(artist.name.isEmpty)
            XCTAssertFalse(artist.topSongs.isEmpty)
            if let albumId = artist.albums.first?.browseId ?? song.album?.id {
                let album = try await api.album(albumId)
                XCTAssertFalse(album.tracks.isEmpty)
            }
            let albums = try await api.search("Whenever You Need Somebody", filter: .playlists)
            if case .playlist(let p)? = albums.items.first {
                let pl = try await api.playlist(p.id)
                XCTAssertFalse(pl.title.isEmpty)
            }
            let details = try await api.track(song.videoId)
            XCTAssertEqual(details.track.videoId, song.videoId)

            let cover = try XCTUnwrap(song.thumbnails.best?.url)
            let img = await core.proto(imageURI(cover, size: 120))
            XCTAssertEqual(img.status, 200)
            XCTAssertGreaterThan(img.body.count, 500)
            let palette = try await api.palette(imageURI(cover))
            XCTAssertNotNil(Palette.rgb(palette.dominant))

            let lyrics = try await api.lyrics(videoId: "dQw4w9WgXcQ", title: "Never Gonna Give You Up",
                                              artist: "Rick Astley", durationMs: 213_000)
            let l = try XCTUnwrap(lyrics)
            XCTAssertFalse(l.lines.isEmpty && (l.plain ?? "").isEmpty)
            print("lyrics from \(l.source), synced: \(l.synced), \(l.lines.count) lines")

            // The logged-out home feed is often refused ("Sorry…", HTTP 403) on a
            // datacenter exit after a burst of requests; the model still has to decode.
            do {
                let home = try await api.home()
                XCTAssertFalse(home.isEmpty)
            } catch let e as CoreError where e.kind == "upstream" {
                print("home feed refused upstream: \(e.message.prefix(80))")
            }
        } catch let e as CoreError where e.kind == "upstream" && e.message.contains("403") {
            // YouTube throttles a datacenter exit after bursts ("Sorry…").
            throw XCTSkip("YouTube Music refused the proxy exit: \(e.message.prefix(60))")
        }
    }

    // MARK: -

    private func firstEvent(_ s: AsyncStream<CoreEvent>, timeout: TimeInterval = 20,
                            where f: @escaping (CoreEvent) -> Bool) async throws -> CoreEvent
    {
        try await withThrowingTaskGroup(of: CoreEvent?.self) { g in
            g.addTask {
                for await e in s where f(e) { return e }
                return nil
            }
            g.addTask {
                try await Task.sleep(nanoseconds: UInt64(timeout * 1e9))
                return nil
            }
            let first = try await g.next() ?? nil
            g.cancelAll()
            return try XCTUnwrap(first, "no matching event in \(timeout) s")
        }
    }
}

final class Recorder<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [T] = []
    func add(_ v: T) { lock.lock(); items.append(v); lock.unlock() }
    var values: [T] { lock.lock(); defer { lock.unlock() }; return items }
}
