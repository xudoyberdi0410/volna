#if os(iOS)
import Foundation
import Observation
import SwiftUI
import VolnaKit
import WebKit

/// The app's state: one core, the player model fed by its events, and the
/// platform glue (AVPlayer, the JS web view, the lock screen). Screens call
/// the core through `api`; nothing here decides what plays.
@MainActor
@Observable
final class AppModel {
    private(set) var core: Core?
    private(set) var api: API?
    private(set) var startError: String?
    var player = PlayerModel()
    /// Colours of the current cover.
    private(set) var palette: Palette?
    /// videoIds the user liked (for the heart buttons).
    private(set) var liked: Set<String> = []
    private(set) var googleSignedIn = false
    /// A short message at the bottom of the screen.
    var toast: String?
    /// Bumped on `library_changed`, so library screens reload.
    private(set) var libraryRevision = 0
    /// Yandex import progress and result (events).
    private(set) var yandexProgress: YandexProgress?
    private(set) var yandexDone: YandexDone?

    @ObservationIgnored let engine = AVEngine()
    @ObservationIgnored private var bridge: PlatformPlayerBridge!
    @ObservationIgnored let js = JSHost()
    @ObservationIgnored private var nowPlaying: NowPlaying!
    @ObservationIgnored private var eventTask: Task<Void, Never>?
    @ObservationIgnored private var paletteFor: String?

    init() {
        bridge = PlatformPlayerBridge(engine: engine) { [weak self] report in
            self?.core?.report(report)
        }
        engine.bridge = bridge
        engine.onResumeAfterInterruption = { [weak self] in self?.run { try await $0.play() } }
        nowPlaying = NowPlaying(app: self)
        start()
    }

    // MARK: - Lifecycle

    /// Where the core keeps its data: Application Support/Volna inside the
    /// app container. It survives app updates and `xtool` reinstalls of the
    /// same bundle id (only deleting the app removes it).
    static var dataDir: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Volna", isDirectory: true)
    }

    static var cacheDir: URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Volna", isDirectory: true)
    }

    func start() {
        let fm = FileManager.default
        try? fm.createDirectory(at: Self.dataDir, withIntermediateDirectories: true)
        try? fm.createDirectory(at: Self.cacheDir, withIntermediateDirectories: true)
        // The core loads the saved Google session (<dataDir>/youtube-cookies.txt) itself.
        let config = CoreConfig(dataDir: Self.dataDir.path, cacheDir: Self.cacheDir.path,
                                audioServer: true, log: "info", codec: "aac")
        do {
            let core = try Core(config: config)
            self.core = core
            api = API(core)
            startError = nil
            core.setPlayerHandler { [weak self] json in
                guard let cmd = PlayerCommand(json) else { return }
                DispatchQueue.main.async { self?.bridge.handle(cmd) }
            }
            core.setJsHandler { [weak self] job in
                DispatchQueue.main.async {
                    guard let self else { return }
                    self.js.run(job) { [weak self] result in
                        self?.core?.jsResult(job.id, result)
                    }
                }
            }
            listen(core)
            Task { await resync(); await refreshLikes(); await refreshGoogleStatus() }
        } catch {
            startError = "\(error)"
        }
    }

    private func listen(_ core: Core) {
        eventTask?.cancel()
        let stream = core.events()
        eventTask = Task { [weak self] in
            for await e in stream {
                guard let self else { return }
                self.handle(e)
            }
        }
    }

    private func handle(_ e: CoreEvent) {
        if let pe = PlayerEvent(e) {
            if !player.apply(pe) {
                Task { await resync() }
            }
            if case .error(let err) = pe, err.action != "retrying" {
                toast = err.text
            }
            playerChanged()
            return
        }
        switch e.name {
        case "library_changed":
            libraryRevision += 1
            if e.payload["kind"].string == "track" { Task { await refreshLikes() } }
        case "yandex_import_progress":
            yandexProgress = try? e.payload.decode(YandexProgress.self)
        case "yandex_import_done":
            yandexDone = try? e.payload.decode(YandexDone.self)
            yandexProgress = nil
            libraryRevision += 1
            Task { await refreshLikes() }
        default:
            break
        }
    }

    func resync() async {
        guard let api else { return }
        if let s = try? await api.snapshot() {
            player.apply(s)
            playerChanged()
        }
    }

    private func playerChanged() {
        nowPlaying.update()
        let cover = player.current?.cover
        if cover != paletteFor {
            paletteFor = cover
            guard let cover, let api else { palette = nil; return }
            Task {
                let p = try? await api.palette(imageURI(cover))
                if self.paletteFor == cover { self.palette = p }
            }
        }
    }

    // MARK: - Actions (thin: each is one core command)

    /// Runs a command and shows its error as a toast.
    func run(_ f: @escaping (API) async throws -> Void) {
        guard let api else { return }
        Task {
            do { try await f(api) } catch { self.toast = Self.describe(error) }
        }
    }

    static func describe(_ error: Error) -> String {
        if let e = error as? CoreError {
            switch e.kind {
            case "upstream": return "YouTube Music не ответил: \(e.message)"
            case "not_found": return "Не найдено"
            case "busy": return "Уже выполняется"
            default: return e.message
            }
        }
        return error.localizedDescription
    }

    func play(_ tracks: [Track], from index: Int = 0) {
        let inputs = tracks.filter(\.playable).map { TrackInput($0, origin: .user) }
        guard !inputs.isEmpty else { return }
        let start = tracks.indices.contains(index)
            ? inputs.firstIndex { $0.id == tracks[index].videoId } ?? 0 : 0
        run { try await $0.setQueue(inputs, startIndex: start, play: true) }
    }

    func enqueue(_ t: Track, at: EnqueueAt) {
        run { try await $0.enqueue([TrackInput(t, origin: .user)], at: at) }
        toast = at == .next ? "Будет следующим" : "Добавлено в очередь"
    }

    func toggle() { run { try await $0.toggle() } }
    func next() { run { try await $0.next() } }
    func prev() { run { try await $0.prev() } }
    func seek(_ seconds: Double) { run { try await $0.seek(seconds) } }

    func startWave() {
        run { api in
            let status = try await api.waveStatus()
            if status.cold { throw CoreError(kind: "cold", message: "Сначала выберите исполнителей") }
            try await api.waveStart()
        }
    }

    func stopWave() { run { try await $0.waveStop() } }

    func dislikeCurrent() { run { try await $0.waveDislike() } }

    func isLiked(_ videoId: String?) -> Bool { videoId.map(liked.contains) ?? false }

    func toggleLike(_ t: Track) {
        let on = !liked.contains(t.videoId)
        if on { liked.insert(t.videoId) } else { liked.remove(t.videoId) }
        run { try await $0.like(.track(t), on: on) }
    }

    func likeCurrent() {
        guard let t = player.current else { return }
        toggleLike(t.asTrack)
    }

    func refreshLikes() async {
        guard let api, let rows = try? await api.library(.track) else { return }
        liked = Set(rows.compactMap { if case .track(let t) = $0.item { t.videoId } else { nil } })
    }

    // MARK: - Google session

    private(set) var googleAccount: String?

    func refreshGoogleStatus() async {
        guard let api, let s = try? await api.youtubeAuthStatus() else { return }
        googleSignedIn = s.signedIn
        googleAccount = s.account
    }

    /// Hands the sign-in web view's session to the core (it saves it).
    func setGoogleCookies(_ text: String, quiet: Bool = false) {
        guard let api else { return }
        Task {
            do {
                let s = try await api.youtubeSetCookies(text)
                googleSignedIn = s.signedIn
                googleAccount = s.account
                if !quiet { toast = s.signedIn ? "Вход выполнен" : "YouTube не принял сессию" }
            } catch {
                if !quiet { toast = Self.describe(error) }
            }
        }
    }

    /// Google rotates the short-lived cookies while the web view is used:
    /// send the store's current ones again (on return to the app).
    func reexportGoogleCookies() {
        guard googleSignedIn else { return }
        Task {
            let cookies = await WKWebsiteDataStore.default().httpCookieStore.allCookies()
            let text = NetscapeCookies.format(cookies)
            if NetscapeCookies.signedIn(text), text != lastExport {
                lastExport = text
                setGoogleCookies(text, quiet: true)
            }
        }
    }

    @ObservationIgnored private var lastExport: String?

    func signOutGoogle() {
        googleSignedIn = false
        googleAccount = nil
        run { _ = try await $0.youtubeSetCookies("") }
    }
}
#endif
