#if os(iOS)
import SwiftUI
import VolnaKit
import WebKit

struct SettingsView: View {
    @Environment(AppModel.self) private var app
    @AppStorage("theme") private var theme = "system"
    @State private var quality = "high"

    var body: some View {
        Form {
            Section {
                NavigationLink {
                    GoogleSignInView()
                } label: {
                    LabeledContent {
                        Text(app.googleSignedIn ? "Выполнен" : "Не выполнен")
                    } label: {
                        Label("Вход в Google", systemImage: "person.crop.circle")
                    }
                }
            } footer: {
                Text("YouTube отдаёт звук только вошедшим пользователям. Сессия хранится на телефоне.")
            }
            Section("Музыка") {
                NavigationLink { WaveSettingsView() } label: { Label("Моя волна", systemImage: "dot.radiowaves.left.and.right") }
                Picker(selection: $quality) {
                    Text("Высокое").tag("high")
                    Text("Экономное").tag("low")
                } label: {
                    Label("Качество", systemImage: "waveform")
                }
                .onChange(of: quality) { _, q in app.run { _ = try await $0.setPlayerSettings(["quality": .string(q)]) } }
                NavigationLink { ScrobbleSettingsView() } label: { Label("Скробблинг", systemImage: "music.note.house") }
                NavigationLink { YandexImportView() } label: { Label("Импорт из Яндекс Музыки", systemImage: "square.and.arrow.down") }
            }
            Section("Оформление") {
                Picker(selection: $theme) {
                    Text("Как в системе").tag("system")
                    Text("Светлое").tag("light")
                    Text("Тёмное").tag("dark")
                } label: {
                    Label("Тема", systemImage: "circle.lefthalf.filled")
                }
            }
            Section("О приложении") {
                LabeledContent("Версия", value: Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "–")
                LabeledContent("Данные") {
                    Text(AppModel.dataDir.path).font(.caption2).foregroundStyle(.secondary).textSelection(.enabled)
                }
                if let codec = app.player.state.stream?.codec {
                    LabeledContent("Поток", value: codec.uppercased())
                }
            }
        }
        .navigationTitle("Настройки")
        .task { if let s = try? await app.api?.snapshot() { quality = s.settings.quality } }
    }
}

// MARK: - Google

/// Google sign-in in a web view. After Google's page, music.youtube.com is
/// loaded once so YouTube gets its copy of the session on `.youtube.com`;
/// then the store's cookies go to the core (`youtube_set_cookies`).
struct GoogleSignInView: View {
    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    @State private var checking = false
    @State private var visitedYouTube = false

    static let start = URL(string: "https://accounts.google.com/ServiceLogin?service=youtube&hl=ru&continue=https%3A%2F%2Fmusic.youtube.com%2F")!
    static let youtube = URL(string: "https://music.youtube.com/")!

    var body: some View {
        VStack(spacing: 0) {
            if app.googleSignedIn {
                HStack {
                    Label(app.googleAccount ?? "Вход выполнен", systemImage: "checkmark.seal.fill").foregroundStyle(.green)
                    Spacer()
                    Button("Выйти", role: .destructive) {
                        WKWebsiteDataStore.default().removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(),
                                                                modifiedSince: .distantPast) {}
                        app.signOutGoogle()
                    }
                }
                .padding()
            }
            WebView(url: Self.start, onNavigate: { url in
                Task { await export(auto: true) }
                // Signed in to Google but not on YouTube yet: go there once.
                let host = url.host ?? ""
                if !visitedYouTube, host.hasSuffix("google.com"), !host.hasPrefix("accounts.") {
                    visitedYouTube = true
                    return Self.youtube
                }
                if host.hasSuffix("youtube.com") { visitedYouTube = true }
                return nil
            })
        }
        .navigationTitle("Вход в Google")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Готово") { Task { await export(auto: false) } }.disabled(checking)
            }
        }
    }

    /// Hands the session over once YouTube has it (`LOGIN_INFO` + SAPISID).
    private func export(auto: Bool) async {
        checking = true
        defer { checking = false }
        let cookies = await WKWebsiteDataStore.default().httpCookieStore.allCookies()
        let text = NetscapeCookies.format(cookies)
        if NetscapeCookies.signedIn(text) {
            app.setGoogleCookies(text)
            dismiss()
        } else if !auto {
            app.toast = "Сначала войдите в аккаунт Google"
        }
    }
}

/// A web view with a "page finished" callback.
struct WebView: UIViewRepresentable {
    let url: URL
    var store: WKWebsiteDataStore = .default()
    /// A page finished loading; return a URL to go there next.
    var onNavigate: (URL) -> URL? = { _ in nil }
    /// Return true to stop the navigation (e.g. a redirect with a token).
    var intercept: (URL) -> Bool = { _ in false }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = store
        let v = WKWebView(frame: .zero, configuration: config)
        // Google refuses sign-in in web views that do not look like Safari.
        v.customUserAgent = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1"
        v.navigationDelegate = context.coordinator
        v.load(URLRequest(url: url))
        return v
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {}

    final class Coordinator: NSObject, WKNavigationDelegate {
        let parent: WebView
        init(_ p: WebView) { parent = p }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            if let u = webView.url, let next = parent.onNavigate(u) {
                webView.load(URLRequest(url: next))
            }
        }

        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                     decisionHandler: @escaping (WKNavigationActionPolicy) -> Void)
        {
            if let u = action.request.url, parent.intercept(u) {
                decisionHandler(.cancel)
            } else {
                decisionHandler(.allow)
            }
        }

        func webView(_ webView: WKWebView, didReceiveServerRedirectForProvisionalNavigation navigation: WKNavigation!) {
            if let u = webView.url { _ = parent.intercept(u) }
        }
    }
}

// MARK: - Scrobbling

struct ScrobbleSettingsView: View {
    @Environment(AppModel.self) private var app
    @Environment(\.openURL) private var openURL
    @State private var status: ScrobbleStatus?
    @State private var tokens: [ScrobbleService: String] = [:]
    @State private var urls: [ScrobbleService: String] = [:]

    var body: some View {
        Form {
            if let s = status {
                Section {
                    Stepper("После \(s.percentage ?? 50)% трека", value: Binding(
                        get: { s.percentage ?? 50 },
                        set: { p in update { try await $0.scrobbleSet(nil, percentage: p) } }
                    ), in: 10...90, step: 5)
                } footer: {
                    Text("Или после 4 минут, что наступит раньше.")
                }
                ForEach(ScrobbleService.allCases) { svc in
                    section(svc, s.of(svc))
                }
            } else {
                ProgressView()
            }
        }
        .navigationTitle("Скробблинг")
        .task { status = try? await app.api?.scrobbleStatus() }
    }

    @ViewBuilder
    private func section(_ svc: ScrobbleService, _ st: ScrobbleStatus.Service?) -> some View {
        Section {
            Toggle("Включено", isOn: Binding(get: { st?.enabled ?? false },
                                            set: { on in update { try await $0.scrobbleSet(svc, enabled: on) } }))
            if st?.canLove ?? (svc != .maloja) {
                Toggle("«Нравится» → love", isOn: Binding(get: { st?.loveOnLike ?? false },
                                                       set: { on in update { try await $0.scrobbleSet(svc, loveOnLike: on) } }))
            }
            switch svc {
            case .lastfm, .librefm:
                if let user = st?.username {
                    LabeledContent("Аккаунт", value: user)
                    Button("Выйти", role: .destructive) { update { try await $0.scrobbleLogout(svc) } }
                } else if st?.loginPending == true {
                    Button("Я разрешил доступ") {
                        update { api in
                            _ = try await api.scrobbleLoginFinish(svc)
                            return try await api.scrobbleStatus()
                        }
                    }
                } else {
                    Button("Войти через браузер") {
                        Task {
                            do {
                                if let u = try await app.api?.scrobbleLoginStart(svc) { openURL(u) }
                                status = try await app.api?.scrobbleStatus()
                            } catch { app.toast = AppModel.describe(error) }
                        }
                    }
                }
            case .listenbrainz, .maloja:
                if svc == .maloja {
                    TextField("Адрес сервера", text: urlBinding(svc, st?.url ?? ""))
                        .textContentType(.URL).keyboardType(.URL).autocorrectionDisabled().textInputAutocapitalization(.never)
                }
                SecureField(st?.hasToken == true ? "Токен сохранён" : "Токен", text: tokenBinding(svc))
                Button("Сохранить") {
                    let token = tokens[svc].flatMap { $0.isEmpty ? nil : $0 }
                    let url = urls[svc].flatMap { $0.isEmpty ? nil : $0 }
                    update { try await $0.scrobbleSet(svc, token: token, url: url) }
                    tokens[svc] = ""
                }
            }
            if let q = st?.queue, let n = q.pending, n > 0 {
                LabeledContent("В очереди", value: "\(n)")
            }
            if let e = st?.queue?.lastError { Text(e).font(.footnote).foregroundStyle(.red) }
        } header: {
            Text(svc.title)
        }
    }

    private func tokenBinding(_ svc: ScrobbleService) -> Binding<String> {
        Binding(get: { tokens[svc] ?? "" }, set: { tokens[svc] = $0 })
    }

    private func urlBinding(_ svc: ScrobbleService, _ initial: String) -> Binding<String> {
        Binding(get: { urls[svc] ?? initial }, set: { urls[svc] = $0 })
    }

    private func update(_ f: @escaping (API) async throws -> ScrobbleStatus) {
        guard let api = app.api else { return }
        Task {
            do { status = try await f(api) } catch { app.toast = AppModel.describe(error) }
        }
    }
}

// MARK: - Yandex

struct YandexImportView: View {
    @Environment(AppModel.self) private var app
    @State private var status: YandexStatus?
    @State private var token = ""
    @State private var login = false
    @State private var options = YandexImportOptions()

    var body: some View {
        Form {
            Section {
                if status?.signedIn == true {
                    LabeledContent("Аккаунт", value: status?.login ?? "вход выполнен")
                } else {
                    Button { login = true } label: { Label("Войти в Яндекс", systemImage: "person.badge.key") }
                    TextField("Или вставьте токен / ссылку с access_token", text: $token, axis: .vertical)
                        .autocorrectionDisabled().textInputAutocapitalization(.never)
                    Button("Сохранить токен") { setToken(token) }.disabled(token.isEmpty)
                }
            } footer: {
                Text("Яндекс Музыка: лайки, плейлисты, альбомы и исполнители переносятся в библиотеку Volna.")
            }
            Section("Что перенести") {
                Toggle("Любимые треки", isOn: $options.likes)
                Toggle("Плейлисты", isOn: $options.playlists)
                Toggle("Альбомы", isOn: $options.albums)
                Toggle("Исполнители", isOn: $options.artists)
            }
            Section {
                if let p = app.yandexProgress {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(stage(p.stage)).font(.subheadline)
                        if let f = p.fraction { ProgressView(value: f) } else { ProgressView() }
                        if let c = p.current { Text(c).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                    }
                    Button("Остановить", role: .destructive) { app.run { _ = try await $0.yandexImportCancel() } }
                } else {
                    Button("Начать импорт") { app.run { [options] in try await $0.yandexImportStart(options) } }
                        .disabled(status?.signedIn != true)
                }
                if let d = app.yandexDone {
                    if d.needLogin == true {
                        Text("Нужно войти в Яндекс").foregroundStyle(.orange)
                    } else if let e = d.error {
                        Text(e).foregroundStyle(.red)
                    } else if let s = d.summary {
                        Text("Найдено \(s.matched ?? 0) из \(s.total ?? 0); треков \(s.likedTracks ?? 0), плейлистов \(s.playlists ?? 0), альбомов \(s.albums ?? 0), исполнителей \(s.artists ?? 0)")
                            .font(.footnote)
                    }
                }
            }
        }
        .navigationTitle("Яндекс Музыка")
        .task { status = try? await app.api?.yandexStatus() }
        .sheet(isPresented: $login) {
            NavigationStack {
                WebView(url: Yandex.authorizeURL(), store: .nonPersistent(), intercept: { url in
                    guard let t = Yandex.token(fromRedirect: url.absoluteString) else { return false }
                    DispatchQueue.main.async {
                        login = false
                        setToken(t)
                    }
                    return true
                })
                .navigationTitle("Вход в Яндекс")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Отмена") { login = false } } }
            }
        }
    }

    private func setToken(_ t: String) {
        guard let api = app.api else { return }
        Task {
            do {
                status = try await api.yandexSetToken(t)
                token = ""
            } catch { app.toast = AppModel.describe(error) }
        }
    }

    private func stage(_ s: String?) -> String {
        switch s {
        case "account": "Аккаунт"
        case "likes": "Читаю лайки"
        case "playlists": "Читаю плейлисты"
        case "albums": "Читаю альбомы"
        case "tracks": "Ищу треки в YouTube Music"
        case "album_match": "Ищу альбомы"
        case "artist_match": "Ищу исполнителей"
        case "saving": "Сохраняю"
        default: "Импорт…"
        }
    }
}
#endif
