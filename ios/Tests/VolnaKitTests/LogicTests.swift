import Foundation
import XCTest
@testable import VolnaKit

/// Pure logic: wire decoding, the player reducer, the platform-player
/// bridge, JS jobs.
final class WireTests: XCTestCase {
    func testJSONRoundTripsAndLiterals() throws {
        let v: JSON = ["a": 1, "b": [true, nil, "x"], "c": 1.5, "d": ["e": "ж"]]
        XCTAssertEqual(try JSON.parse(v.encoded()), v)
        XCTAssertEqual(v.encoded(), #"{"a":1,"b":[true,null,"x"],"c":1.5,"d":{"e":"ж"}}"#)
        XCTAssertEqual(v["b"][2].string, "x")
        XCTAssertEqual(v["missing"]["deeper"], .null)
    }

    func testCoreErrorSplitsKind() {
        XCTAssertEqual(CoreError(wire: "not_found: no playlist x"), CoreError(kind: "not_found", message: "no playlist x"))
        XCTAssertEqual(CoreError(wire: "invalid args `id`: missing").kind, "other")
        XCTAssertEqual(CoreError(wire: "boom").message, "boom")
    }

    func testSnakeAndCamelKeysDecode() throws {
        let row: JSON = [
            "item": ["type": "album", "browse_id": "MPREb_1", "playlist_id": "OLAK", "title": "T",
                     "kind": "ep", "artists": [["id": nil, "name": "A"]], "thumbnails": [], "explicit": false],
            "addedAt": 1_700_000_000_000,
        ]
        let liked = try row.decode(Liked.self)
        guard case .album(let a) = liked.item else { return XCTFail() }
        XCTAssertEqual(a.browseId, "MPREb_1")
        XCTAssertEqual(a.playlistId, "OLAK")
        XCTAssertEqual(a.kind, .ep)
        XCTAssertEqual(liked.addedAt, 1_700_000_000_000)
    }

    func testUnknownItemTypesAreDropped() throws {
        let page: JSON = ["top_result": ["type": "episode", "id": "x"],
                          "items": [["type": "podcast"], ["type": "artist", "id": "UC1", "name": "N"]],
                          "continuation": nil]
        let p = try page.decode(SearchPage.self)
        XCTAssertNil(p.topResult)
        XCTAssertEqual(p.items.map(\.id), ["r:UC1"])
    }

    func testTracksEncodeBackSnakeCase() throws {
        let t = Track(videoId: "dQw4w9WgXcQ", title: "T", durationS: 3)
        let j = API.snake(t)
        XCTAssertEqual(j["video_id"].string, "dQw4w9WgXcQ")
        XCTAssertEqual(j["duration_s"].int, 3)
        let item = API.snake(Item.track(t))
        XCTAssertEqual(item["type"].string, "track")
        XCTAssertEqual(try item.decode(Item.self), .track(t))
    }

    func testTrackInputFromCatalogTrack() throws {
        let t = Track(videoId: "dQw4w9WgXcQ", title: "T", artists: [ArtistRef(id: "UC", name: "A")],
                      album: AlbumRef(id: nil, name: "Al"), durationS: 200,
                      thumbnails: [Thumbnail(url: "s", width: 60, height: 60), Thumbnail(url: "l", width: 544, height: 544)],
                      kind: .officialVideo)
        let j = try JSON(encoding: TrackInput(t, origin: .user))
        XCTAssertEqual(j["cover"].string, "l")
        XCTAssertEqual(j["kind"].string, "video")
        XCTAssertEqual(j["album"]["title"].string, "Al")
        XCTAssertEqual(j["duration"].double, 200)
        XCTAssertEqual(j["origin"].string, "user")
    }

    func testImageURIEncodesTheURL() {
        XCTAssertEqual(imageURI("https://lh3.googleusercontent.com/a=w60-h60?x=1&y", size: 120),
                       "volna://img/?u=https%3A%2F%2Flh3.googleusercontent.com%2Fa%3Dw60-h60%3Fx%3D1%26y&s=120")
        XCTAssertEqual(imageURI("volna://img/?u=x"), "volna://img/?u=x")
    }

    func testYandexTokenFromRedirect() {
        XCTAssertEqual(Yandex.token(fromRedirect: "https://oauth.yandex.ru/verification_code#access_token=y0_AbC&token_type=bearer"), "y0_AbC")
        XCTAssertNil(Yandex.token(fromRedirect: "https://oauth.yandex.ru/authorize"))
    }

    func testLyricsLineLookup() throws {
        let j: JSON = ["source": "lrclib", "synced": "line", "lines": [
            ["startMs": 1000, "endMs": 2000, "text": "a"],
            ["startMs": 2000, "endMs": 3000, "text": "b"],
            ["startMs": 5000, "endMs": 6000, "text": "c", "words": [["startMs": 5000, "endMs": 5500, "text": "c"]]],
        ]]
        let l = try j.decode(Lyrics.self)
        XCTAssertNil(l.lineIndex(at: 500))
        XCTAssertEqual(l.lineIndex(at: 1000), 0)
        XCTAssertEqual(l.lineIndex(at: 4000), 1)
        XCTAssertEqual(l.lineIndex(at: 99_000), 2)
        XCTAssertEqual(l.lines[2].words?.count, 1)
    }

    func testPaletteHex() {
        let (r, g, b) = Palette.rgb("#ff8000")!
        XCTAssertEqual(r, 1)
        XCTAssertEqual(g, 128.0 / 255, accuracy: 1e-9)
        XCTAssertEqual(b, 0)
        XCTAssertNil(Palette.rgb("nope"))
    }
}

final class PlayerModelTests: XCTestCase {
    func track(_ uid: String) -> JSON {
        ["uid": .string(uid), "id": "dQw4w9WgXcQ", "title": .string("t\(uid)"), "artists": [],
         "album": nil, "duration": 100, "cover": nil, "explicit": false, "kind": "track",
         "origin": "wave", "raw": nil, "ui": nil, "unavailable": false]
    }

    func snapshot() throws -> PlayerSnapshot {
        try ([
            "protocol": 1,
            "state": ["status": "paused", "uid": "1", "trackId": "dQw4w9WgXcQ", "position": 10, "at": 1000,
                      "duration": 100, "rate": 1, "buffering": false, "crossfading": false, "volume": 0.7,
                      "muted": false, "stream": nil, "error": nil],
            "queue": ["rev": 5, "items": [track("1"), track("2")], "original": nil, "index": 0,
                      "shuffle": false, "repeat": "off", "continuation": ["kind": "wave", "fetching": false]],
            "settings": ["volume": 0.7, "muted": false, "quality": "high", "autoplay": false],
            "dsp": [:], "sleepTimer": nil, "wave": ["active": true], "journalSeq": 3,
        ] as JSON).decode(PlayerSnapshot.self)
    }

    func event(_ name: String, _ payload: JSON) -> PlayerEvent {
        PlayerEvent(CoreEvent(name: name, payload: payload))!
    }

    func testSnapshotThenQueueEvents() throws {
        var m = PlayerModel()
        m.apply(try snapshot())
        XCTAssertEqual(m.current?.uid, "1")
        XCTAssertEqual(m.upNext.map(\.uid), ["2"])
        XCTAssertTrue(m.waveActive)
        XCTAssertEqual(m.continuation.kind, .wave)

        // Stale revisions are ignored.
        XCTAssertTrue(m.apply(event("player:queue", ["rev": 5, "order": ["2"], "original": nil, "index": 0,
            "shuffle": false, "repeat": "off", "continuation": ["kind": "wave", "fetching": false],
            "added": [], "updated": []])))
        XCTAssertEqual(m.order, ["1", "2"])

        // The next one adds an entry and drops one.
        XCTAssertTrue(m.apply(event("player:queue", ["rev": 6, "order": ["2", "3"], "original": nil, "index": 0,
            "shuffle": true, "repeat": "all", "continuation": ["kind": "wave", "fetching": true],
            "added": [track("3")], "updated": []])))
        XCTAssertEqual(m.queue.map(\.uid), ["2", "3"])
        XCTAssertNil(m.items["1"])
        XCTAssertTrue(m.shuffle)
        XCTAssertEqual(m.repeat, .all)

        // A gap asks for a snapshot.
        XCTAssertFalse(m.apply(event("player:queue", ["rev": 9, "order": ["2", "3"], "original": nil, "index": 1,
            "shuffle": false, "repeat": "off", "continuation": ["kind": "none", "fetching": false],
            "added": [], "updated": []])))
        // So does an entry it never saw.
        XCTAssertFalse(m.apply(event("player:queue", ["rev": 10, "order": ["2", "9"], "original": nil, "index": 1,
            "shuffle": false, "repeat": "off", "continuation": ["kind": "none", "fetching": false],
            "added": [], "updated": []])))
    }

    func testStatePositionAndExtrapolation() throws {
        var m = PlayerModel()
        m.apply(try snapshot())
        XCTAssertEqual(m.position(atMs: 5000), 10) // paused: no drift
        m.apply(event("player:state", ["status": "playing", "uid": "1", "trackId": "dQw4w9WgXcQ", "position": 20,
                                       "at": 10_000, "duration": 100, "rate": 1, "buffering": false, "crossfading": false,
                                       "volume": 0.7, "muted": false, "stream": ["codec": "aac", "bitrateKbps": 128,
                                                                                 "quality": "HIGH", "mime": "audio/mp4"],
                                       "error": nil]))
        XCTAssertEqual(m.state.stream?.codec, "aac")
        XCTAssertEqual(m.position(atMs: 12_500), 22.5, accuracy: 1e-9)
        XCTAssertEqual(m.position(atMs: 1_000_000), 100) // clamped to the duration
        m.apply(event("player:position", ["uid": "other", "position": 50, "duration": 100, "rate": 1, "at": 11_000, "playing": true]))
        XCTAssertEqual(m.state.position, 20, "another entry's position is ignored")
        m.apply(event("player:position", ["uid": "1", "position": 30, "duration": 100, "rate": 1, "at": 20_000, "playing": true]))
        XCTAssertEqual(m.position(atMs: 21_000), 31, accuracy: 1e-9)
        m.apply(event("player:wave", ["active": false]))
        XCTAssertFalse(m.waveActive)
        m.apply(event("player:sleep", ["endsAt": 123]))
        XCTAssertEqual(m.sleepEndsAt, 123)
        m.apply(event("player:error", ["uid": "1", "trackId": "dQw4w9WgXcQ", "kind": "unavailable",
                                       "message": "x", "action": "skipped", "streak": 1]))
        XCTAssertEqual(m.lastError?.text, "Трек недоступен")
        XCTAssertNil(PlayerEvent(CoreEvent(name: "library_changed", payload: [:])))
    }
}

final class PlatformPlayerTests: XCTestCase {
    final class FakeEngine: MediaEngine {
        var calls: [String] = []
        var lastToken = 0
        func load(url: URL, startSeconds: Double, token: Int) {
            calls.append("load \(url.absoluteString) \(startSeconds)")
            lastToken = token
        }
        func play() { calls.append("play") }
        func pause() { calls.append("pause") }
        func seek(seconds: Double) { calls.append("seek \(seconds)") }
        func stop() { calls.append("stop") }
        func setVolume(_ v: Double) { calls.append("volume \(v)") }
    }

    func testCommandsParseFromTheCoreJSON() {
        XCTAssertEqual(PlayerCommand(["op": "load", "url": "http://127.0.0.1:5/t/stream/dQw4w9WgXcQ?codec=aac", "startMs": 1500]),
                       .load(url: URL(string: "http://127.0.0.1:5/t/stream/dQw4w9WgXcQ?codec=aac")!, startMs: 1500))
        XCTAssertEqual(PlayerCommand(["op": "seek", "positionMs": 42500]), .seek(positionMs: 42500))
        XCTAssertEqual(PlayerCommand(["op": "setVolume", "volume": 0.5]), .setVolume(0.5))
        XCTAssertEqual(PlayerCommand(["op": "play"]), .play)
        XCTAssertNil(PlayerCommand(["op": "dance"]))
        XCTAssertEqual(PlayerReport.position(ms: 5, durationMs: nil).json.encoded(),
                       #"{"durationMs":null,"positionMs":5,"type":"position"}"#)
        XCTAssertEqual(PlayerReport.ready(durationMs: 180_000).json.encoded(), #"{"durationMs":180000,"type":"ready"}"#)
    }

    func testReportsAreAboutTheLatestLoadOnly() {
        let engine = FakeEngine()
        var reports: [PlayerReport] = []
        let b = PlatformPlayerBridge(engine: engine) { reports.append($0) }
        b.handle(.load(url: URL(string: "http://127.0.0.1:1/a")!, startMs: 2000))
        XCTAssertEqual(engine.calls, ["load http://127.0.0.1:1/a 2.0"])
        let first = engine.lastToken
        b.enginePosition(token: first, seconds: 1) // not ready yet
        b.engineReady(token: first, duration: 180)
        b.engineReady(token: first, duration: 180) // once
        b.handle(.play)
        b.enginePlaying(token: first)
        b.enginePosition(token: first, seconds: 2.25)
        b.engineBuffering(token: first, true)
        b.enginePlaying(token: first)
        b.handle(.pause)
        b.enginePaused(token: first)
        b.enginePosition(token: first, seconds: 3) // paused: no position
        XCTAssertEqual(reports, [.ready(durationMs: 180_000), .playing, .position(ms: 2250, durationMs: 180_000),
                                 .buffering(true), .buffering(false), .paused])

        reports = []
        b.handle(.stop)
        b.handle(.load(url: URL(string: "http://127.0.0.1:1/b")!, startMs: 0))
        let second = engine.lastToken
        XCTAssertNotEqual(first, second)
        b.engineEnded(token: first) // the old item: dropped
        b.engineFailed(token: first, message: "late")
        b.engineReady(token: second, duration: nil)
        b.handle(.play)
        b.enginePlaying(token: second)
        b.engineEnded(token: second)
        b.engineEnded(token: second)
        b.enginePaused(token: second) // AVPlayer pauses at the end: not a pause
        XCTAssertEqual(reports, [.ready(durationMs: nil), .playing, .ended])
        b.handle(.setVolume(3))
        XCTAssertEqual(engine.calls.last, "volume 1.0")
    }

    func testErrorsAndSeeksAfterTheEnd() {
        let engine = FakeEngine()
        var reports: [PlayerReport] = []
        let b = PlatformPlayerBridge(engine: engine) { reports.append($0) }
        b.handle(.load(url: URL(string: "http://127.0.0.1:1/a")!, startMs: 0))
        b.engineFailed(token: engine.lastToken, message: "HTTP 500", decode: false)
        XCTAssertEqual(reports, [.error(message: "HTTP 500", decode: false)])
        b.handle(.load(url: URL(string: "http://127.0.0.1:1/a")!, startMs: 0))
        let t = engine.lastToken
        b.engineReady(token: t, duration: 10)
        b.handle(.play)
        b.enginePlaying(token: t)
        b.engineEnded(token: t)
        b.handle(.seek(positionMs: 1000)) // repeat one: seek back and play again
        b.handle(.play)
        b.enginePlaying(token: t)
        XCTAssertEqual(Array(reports.suffix(4)), [.ready(durationMs: 10_000), .playing, .ended, .playing])
    }
}

final class JSJobTests: XCTestCase {
    func testJobsParseAndAnswer() {
        let job = JSJob(id: 7, request: ["script": "volnaJs", "args": ["op": "version"]])!
        XCTAssertEqual(job.arguments["script"], "volnaJs")
        XCTAssertEqual(job.arguments["argsJson"], #"{"op":"version"}"#)
        XCTAssertNil(JSJob(id: 1, request: ["args": 1]))
        XCTAssertEqual(JSJob(id: 2, request: ["script": "1+1"])?.arguments["argsJson"], "null")

        XCTAssertEqual(JSJob.answer(evaluated: #"{"a":1}"#, error: nil), .success(#"{"a":1}"#))
        XCTAssertEqual(JSJob.answer(evaluated: nil, error: nil), .success("null"))
        XCTAssertEqual(JSJob.answer(evaluated: "{nope", error: nil), .failure(JSError("the script result is not JSON")))
        let e = NSError(domain: "WKErrorDomain", code: 4, userInfo: ["WKJavaScriptExceptionMessage": "TypeError: x"])
        XCTAssertEqual(JSJob.answer(evaluated: nil, error: e), .failure(JSError("TypeError: x")))
    }

    func testTheCoreGetsJobsAndAnswers() async throws {
        // Without a stream resolver the core never sends jobs; the handler and
        // the answer path are still wired: an unknown id is refused.
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("volnakit-js-\(UUID().uuidString)")
        let core = try Core(config: CoreConfig(dataDir: dir.path, audioServer: false))
        defer { core.shutdown(); try? FileManager.default.removeItem(at: dir) }
        core.setJsHandler { id, _ in _ = id }
        XCTAssertFalse(core.jsResult(1, .failure(JSError("x"))))
    }
}
