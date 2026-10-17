#if os(iOS)
import SwiftUI
import VolnaKit

/// A track in a list: tap plays the list from it; the menu has the rest.
struct TrackRow: View {
    @Environment(AppModel.self) private var app
    let track: Track
    var index: Int?
    var showCover = true
    let play: () -> Void

    var body: some View {
        let current = app.player.current?.trackId == track.videoId
        Button(action: play) {
            HStack(spacing: 12) {
                if showCover {
                    Cover(url: track.thumbnails.best?.url, size: 44, corner: 6)
                } else if let index {
                    Text("\(index + 1)").font(.subheadline.monospacedDigit()).foregroundStyle(.secondary)
                        .frame(width: 24)
                }
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 4) {
                        Text(track.title).font(.body).lineLimit(1)
                            .foregroundStyle(current ? Color.accentColor : .primary)
                        if track.explicit {
                            Image(systemName: "e.square.fill").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    Text(track.artistNames).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 4)
                if current, app.player.isPlaying {
                    Image(systemName: "waveform").symbolEffect(.variableColor.iterative).foregroundStyle(Color.accentColor)
                } else if let d = track.durationS {
                    Text(clock(Double(d))).font(.footnote.monospacedDigit()).foregroundStyle(.secondary)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!track.playable)
        .opacity(track.playable ? 1 : 0.45)
        .contextMenu { TrackMenu(track: track) }
    }
}

/// What can be done with a track.
struct TrackMenu: View {
    @Environment(AppModel.self) private var app
    let track: Track

    var body: some View {
        Button { app.enqueue(track, at: .next) } label: { Label("Играть следующим", systemImage: "text.insert") }
        Button { app.enqueue(track, at: .end) } label: { Label("В конец очереди", systemImage: "text.append") }
        Button { app.toggleLike(track) } label: {
            app.isLiked(track.videoId)
                ? Label("Убрать из любимых", systemImage: "heart.slash")
                : Label("Нравится", systemImage: "heart")
        }
        AddToPlaylistMenu(tracks: [track])
        if let a = track.album?.id {
            NavigationLink(value: Route.album(a)) { Label("Альбом", systemImage: "square.stack") }
        }
        if let r = track.artists.first(where: { $0.id != nil })?.id {
            NavigationLink(value: Route.artist(r)) { Label("Исполнитель", systemImage: "music.mic") }
        }
    }
}

/// "Добавить в плейлист" with the user's playlists.
struct AddToPlaylistMenu: View {
    @Environment(AppModel.self) private var app
    let tracks: [Track]
    @State private var lists: [PlaylistInfo] = []

    var body: some View {
        Menu {
            ForEach(lists) { p in
                Button(p.name) {
                    app.run { _ = try await $0.addToPlaylist(p.id, tracks: tracks) }
                    app.toast = "Добавлено в «\(p.name)»"
                }
            }
            Button {
                app.run { _ = try await $0.createPlaylist(name: "Новый плейлист", tracks: tracks) }
                app.toast = "Создан плейлист «Новый плейлист»"
            } label: { Label("Новый плейлист", systemImage: "plus") }
        } label: {
            Label("Добавить в плейлист", systemImage: "text.badge.plus")
        }
        .task { lists = (try? await app.api?.playlists()) ?? [] }
    }
}

/// An album, artist or playlist in a list (or a track: plays it).
struct ItemRow: View {
    @Environment(AppModel.self) private var app
    let item: Item
    var onPlay: (() -> Void)?

    var body: some View {
        switch item {
        case .track(let t):
            TrackRow(track: t) {
                if let onPlay { onPlay() } else { app.play([t]) }
            }
        case .album(let a):
            NavigationLink(value: Route.album(a.browseId)) { row(round: false) }
        case .artist(let a):
            NavigationLink(value: Route.artist(a.id)) { row(round: true) }
        case .playlist(let p):
            NavigationLink(value: Route.playlist(p.id)) { row(round: false) }
        case .unknown:
            EmptyView()
        }
    }

    private func row(round: Bool) -> some View {
        HStack(spacing: 12) {
            Cover(url: item.thumbnails.best?.url, size: 52, corner: 8, round: round)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title).lineLimit(1)
                Text(item.subtitle).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
            }
        }
    }
}

/// A card in a horizontal shelf.
struct ItemCard: View {
    let item: Item
    var width: CGFloat = 150

    var body: some View {
        let round: Bool = if case .artist = item { true } else { false }
        VStack(alignment: round ? .center : .leading, spacing: 6) {
            Cover(url: item.thumbnails.best?.url, size: width, corner: 10, round: round)
            Text(item.title).font(.subheadline.weight(.medium)).lineLimit(1)
            Text(item.subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1)
        }
        .frame(width: width)
    }
}

struct Shelf: View {
    @Environment(AppModel.self) private var app
    let title: String
    let items: [Item]

    var body: some View {
        if !items.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                Text(title).font(.title3.weight(.bold)).padding(.horizontal)
                ScrollView(.horizontal, showsIndicators: false) {
                    LazyHStack(alignment: .top, spacing: 14) {
                        ForEach(items) { item in
                            switch item {
                            case .track(let t):
                                Button { app.play([t]) } label: { ItemCard(item: item) }.buttonStyle(.plain)
                                    .contextMenu { TrackMenu(track: t) }
                            case .album(let a):
                                NavigationLink(value: Route.album(a.browseId)) { ItemCard(item: item) }.buttonStyle(.plain)
                            case .artist(let a):
                                NavigationLink(value: Route.artist(a.id)) { ItemCard(item: item, width: 120) }.buttonStyle(.plain)
                            case .playlist(let p):
                                NavigationLink(value: Route.playlist(p.id)) { ItemCard(item: item) }.buttonStyle(.plain)
                            case .unknown:
                                EmptyView()
                            }
                        }
                    }
                    .padding(.horizontal)
                }
            }
        }
    }
}

/// Loading / error / content for a screen that loads one thing.
struct Loadable<T, Content: View>: View {
    let load: () async throws -> T
    @ViewBuilder let content: (T) -> Content
    @State private var value: T?
    @State private var error: String?

    var body: some View {
        Group {
            if let value {
                content(value)
            } else if let error {
                ContentUnavailableView {
                    Label("Не загрузилось", systemImage: "wifi.exclamationmark")
                } description: {
                    Text(error)
                } actions: {
                    Button("Повторить") { Task { await fetch() } }
                }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task { if value == nil { await fetch() } }
    }

    private func fetch() async {
        error = nil
        do { value = try await load() } catch { self.error = AppModel.describe(error) }
    }
}
#endif
