#if os(iOS)
import SwiftUI
import VolnaKit

@main
struct VolnaApp: App {
    @State private var app = AppModel()
    @AppStorage("theme") private var theme = "system"
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(app)
                .preferredColorScheme(theme == "dark" ? .dark : theme == "light" ? .light : nil)
                .tint(.accentColor)
                .onChange(of: phase) { _, p in if p == .active { app.reexportGoogleCookies() } }
        }
    }
}

/// Places a pushed screen can go to.
enum Route: Hashable {
    case album(String)
    case artist(String)
    case playlist(String)
    case userPlaylist(String)
    case history
}

extension View {
    /// The catalog screens every tab can push.
    func routes() -> some View {
        navigationDestination(for: Route.self) { r in
            switch r {
            case .album(let id): AlbumView(id: id)
            case .artist(let id): ArtistView(id: id)
            case .playlist(let id): PlaylistView(id: id)
            case .userPlaylist(let id): UserPlaylistView(id: id)
            case .history: HistoryView()
            }
        }
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var app
    @State private var tab = 0
    @State private var fullPlayer = false

    var body: some View {
        @Bindable var app = app
        Group {
            if let error = app.startError {
                ContentUnavailableView {
                    Label("Ядро не запустилось", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(error)
                } actions: {
                    Button("Ещё раз") { app.start() }
                }
            } else {
                TabView(selection: $tab) {
                    Tab("Волна", systemImage: "dot.radiowaves.left.and.right", value: 0) {
                        NavigationStack { WaveView().routes() }
                    }
                    Tab("Поиск", systemImage: "magnifyingglass", value: 1) {
                        NavigationStack { SearchView().routes() }
                    }
                    Tab("Библиотека", systemImage: "music.note.list", value: 2) {
                        NavigationStack { LibraryView().routes() }
                    }
                    Tab("Настройки", systemImage: "gearshape", value: 3) {
                        NavigationStack { SettingsView() }
                    }
                }
                .tabViewBottomAccessory {
                    if app.player.current != nil {
                        MiniPlayer { fullPlayer = true }
                    }
                }
                .sheet(isPresented: $fullPlayer) {
                    FullPlayerView()
                }
            }
        }
        .background(alignment: .topLeading) { JSHostView(host: app.js).frame(width: 2, height: 2) }
        .overlay(alignment: .top) { Toast(text: $app.toast) }
    }
}

/// A short message that goes away by itself.
struct Toast: View {
    @Binding var text: String?

    var body: some View {
        if let t = text {
            Text(t)
                .font(.subheadline.weight(.medium))
                .multilineTextAlignment(.center)
                .padding(.horizontal, 16).padding(.vertical, 10)
                .background(.regularMaterial, in: Capsule())
                .shadow(color: .black.opacity(0.12), radius: 12, y: 4)
                .padding(.top, 8)
                .padding(.horizontal, 24)
                .transition(.move(edge: .top).combined(with: .opacity))
                .task(id: t) {
                    try? await Task.sleep(nanoseconds: 3_000_000_000)
                    withAnimation { text = nil }
                }
                .onTapGesture { withAnimation { text = nil } }
        }
    }
}
#endif
