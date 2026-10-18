#if os(iOS)
import SwiftUI
import VolnaKit

/// Search with suggestions, filters and endless scroll.
struct SearchView: View {
    @Environment(AppModel.self) private var app
    @State private var query = ""
    @State private var submitted = ""
    @State private var filter: SearchFilter = .all
    @State private var suggestions: SearchSuggestions?
    @State private var page: SearchPage?
    @State private var items: [Item] = []
    @State private var loading = false
    @State private var error: String?
    @State private var moods: [MoodCategory] = []

    var body: some View {
        List {
            if submitted.isEmpty {
                if let s = suggestions, !query.isEmpty {
                    ForEach(s.queries, id: \.self) { q in
                        Button { query = q; submit() } label: {
                            Label(q, systemImage: "magnifyingglass").foregroundStyle(.primary)
                        }
                    }
                    ForEach(s.items) { ItemRow(item: $0) }
                } else {
                    browse
                }
            } else {
                Section {
                    Picker("Фильтр", selection: $filter) {
                        ForEach(SearchFilter.allCases, id: \.self) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .listRowSeparator(.hidden)
                }
                if let top = page?.topResult, filter == .all {
                    Section("Лучший результат") { ItemRow(item: top) }
                }
                Section {
                    ForEach(items) { item in
                        ItemRow(item: item, onPlay: playFrom(item))
                            .onAppear { if item.id == items.last?.id { more() } }
                    }
                    if loading { HStack { Spacer(); ProgressView(); Spacer() } }
                    if let error { Text(error).foregroundStyle(.secondary) }
                }
            }
        }
        .listStyle(.plain)
        .navigationTitle("Поиск")
        .searchable(text: $query, prompt: "Треки, альбомы, исполнители")
        .onSubmit(of: .search) { submit() }
        .onChange(of: query) { _, q in if q.isEmpty { submitted = ""; items = []; page = nil } }
        .onChange(of: filter) { _, _ in if !submitted.isEmpty { run(submitted) } }
        .task(id: query) {
            guard !query.isEmpty, submitted != query else { return }
            try? await Task.sleep(nanoseconds: 250_000_000)
            suggestions = try? await app.api?.suggestions(query)
        }
        .task { if moods.isEmpty { moods = (try? await app.api?.moods()) ?? [] } }
    }

    @ViewBuilder private var browse: some View {
        ForEach(moods) { cat in
            Section(cat.title) {
                ForEach(cat.moods.prefix(12)) { mood in
                    NavigationLink {
                        MoodView(mood: mood)
                    } label: {
                        HStack {
                            RoundedRectangle(cornerRadius: 4).fill(moodColor(mood)).frame(width: 6, height: 28)
                            Text(mood.title)
                        }
                    }
                }
            }
        }
    }

    private func moodColor(_ m: MoodCategory.Mood) -> Color {
        guard let c = m.color else { return .accentColor }
        return Color(red: Double((c >> 16) & 0xff) / 255, green: Double((c >> 8) & 0xff) / 255, blue: Double(c & 0xff) / 255)
    }

    /// A track plays the search's tracks from it.
    private func playFrom(_ item: Item) -> (() -> Void)? {
        guard case .track(let t) = item else { return nil }
        return {
            let tracks = items.compactMap { if case .track(let x) = $0 { x } else { nil } }
            app.play(tracks, from: tracks.firstIndex(of: t) ?? 0)
        }
    }

    private func submit() {
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        submitted = query
        run(query)
    }

    private func run(_ q: String) {
        loading = true
        error = nil
        items = []
        page = nil
        let f = filter
        Task {
            do {
                let p = try await app.api?.search(q, filter: f)
                guard q == submitted, f == filter else { return }
                page = p
                items = p?.items ?? []
            } catch { self.error = AppModel.describe(error) }
            loading = false
        }
    }

    private func more() {
        guard !loading, filter != .all, let c = page?.continuation else { return }
        loading = true
        let f = filter
        Task {
            if let p = try? await app.api?.searchMore(c, filter: f), f == filter {
                page?.continuation = p.continuation
                items += p.items
            } else {
                page?.continuation = nil
            }
            loading = false
        }
    }
}

struct MoodView: View {
    @Environment(AppModel.self) private var app
    let mood: MoodCategory.Mood

    var body: some View {
        Loadable(load: { try await app.api?.mood(mood.params) ?? [] }) { sections in
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    ForEach(sections) { Shelf(title: $0.title, items: $0.items) }
                }
                .padding(.vertical)
            }
        }
        .navigationTitle(mood.title)
    }
}

/// The header of an album / playlist / artist page.
struct Header: View {
    let cover: String?
    let title: String
    let subtitle: String
    var round = false
    let play: (() -> Void)?
    let shuffle: (() -> Void)?

    var body: some View {
        VStack(spacing: 14) {
            Cover(url: cover, size: 220, corner: 14, round: round)
                .shadow(color: .black.opacity(0.2), radius: 18, y: 8)
            VStack(spacing: 4) {
                Text(title).font(.title2.weight(.bold)).multilineTextAlignment(.center)
                Text(subtitle).font(.subheadline).foregroundStyle(.secondary).multilineTextAlignment(.center)
            }
            HStack(spacing: 12) {
                if let play {
                    Button(action: play) { Label("Слушать", systemImage: "play.fill").frame(maxWidth: .infinity) }
                        .buttonStyle(.borderedProminent)
                }
                if let shuffle {
                    Button(action: shuffle) { Label("Вперемешку", systemImage: "shuffle").frame(maxWidth: .infinity) }
                        .buttonStyle(.bordered)
                }
            }
            .controlSize(.large)
        }
        .frame(maxWidth: .infinity)
        .listRowSeparator(.hidden)
        .padding(.vertical, 8)
    }
}

struct AlbumView: View {
    @Environment(AppModel.self) private var app
    let id: String

    var body: some View {
        Loadable(load: { try await app.api!.album(id) }) { a in
            List {
                Header(cover: a.thumbnails.best?.url, title: a.title,
                       subtitle: ([a.kind.label, a.artists.map(\.name).joined(separator: ", "), a.year.map(String.init) ?? ""]
                           .filter { !$0.isEmpty }).joined(separator: " · "),
                       play: { app.play(a.tracks) },
                       shuffle: { app.play(a.tracks.shuffled()) })
                ForEach(Array(a.tracks.enumerated()), id: \.offset) { i, t in
                    TrackRow(track: t, index: i, showCover: false) { app.play(a.tracks, from: i) }
                }
                if !a.related.isEmpty {
                    Shelf(title: "Ещё", items: a.related.map(Item.album))
                        .listRowInsets(EdgeInsets()).listRowSeparator(.hidden)
                }
            }
            .listStyle(.plain)
            .toolbar {
                ToolbarItem {
                    Button { app.run { _ = try await $0.like(.album(AlbumSummaryFrom(a)), on: true) }
                        app.toast = "Альбом в библиотеке"
                    } label: { Image(systemName: "plus.circle") }
                    .accessibilityLabel("В библиотеку")
                }
            }
        }
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// An album page as a summary (what the library stores).
func AlbumSummaryFrom(_ a: Album) -> AlbumSummary {
    // The core takes the album model; the summary fields are a subset of it.
    let j: JSON = [
        "browse_id": .string(a.browseId), "playlist_id": JSON(a.playlistId), "title": .string(a.title),
        "kind": .string(a.kind.rawValue),
        "artists": .array(a.artists.map { ["id": JSON($0.id), "name": .string($0.name)] }),
        "year": JSON(a.year),
        "thumbnails": .array(a.thumbnails.map { ["url": .string($0.url), "width": JSON($0.width), "height": JSON($0.height)] }),
        "explicit": .bool(a.explicit),
    ]
    return try! j.decode(AlbumSummary.self)
}

struct ArtistView: View {
    @Environment(AppModel.self) private var app
    let id: String

    var body: some View {
        Loadable(load: { try await app.api!.artist(id) }) { a in
            List {
                Header(cover: a.thumbnails.best?.url, title: a.name, subtitle: a.subscribers ?? "", round: true,
                       play: a.topSongs.isEmpty ? nil : { app.play(a.topSongs) },
                       shuffle: a.topSongs.isEmpty ? nil : { app.play(a.topSongs.shuffled()) })
                if !a.topSongs.isEmpty {
                    Section("Популярное") {
                        ForEach(Array(a.topSongs.prefix(10).enumerated()), id: \.offset) { i, t in
                            TrackRow(track: t) { app.play(a.topSongs, from: i) }
                        }
                    }
                }
                Group {
                    Shelf(title: "Альбомы", items: a.albums.map(Item.album))
                    Shelf(title: "Синглы и EP", items: a.singles.map(Item.album))
                    Shelf(title: "Плейлисты", items: a.playlists.map(Item.playlist))
                    Shelf(title: "Похожие", items: a.related.map(Item.artist))
                }
                .listRowInsets(EdgeInsets(top: 12, leading: 0, bottom: 12, trailing: 0))
                .listRowSeparator(.hidden)
                if let d = a.description, !d.isEmpty {
                    Section("Об исполнителе") { Text(d).font(.callout).foregroundStyle(.secondary) }
                }
            }
            .listStyle(.plain)
        }
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct PlaylistView: View {
    @Environment(AppModel.self) private var app
    let id: String
    @State private var extra: [Track] = []
    @State private var continuation: String?
    @State private var started = false

    var body: some View {
        Loadable(load: { try await app.api!.playlist(id) }) { p in
            let tracks = p.tracks + extra
            List {
                Header(cover: p.thumbnails.best?.url, title: p.title,
                       subtitle: [p.author?.name, p.trackCount.map { "\($0) треков" }].compactMap { $0 }.joined(separator: " · "),
                       play: { app.play(tracks) }, shuffle: { app.play(tracks.shuffled()) })
                ForEach(Array(tracks.enumerated()), id: \.offset) { i, t in
                    TrackRow(track: t) { app.play(tracks, from: i) }
                        .onAppear { if i == tracks.count - 1 { more(p) } }
                }
            }
            .listStyle(.plain)
        }
        .navigationBarTitleDisplayMode(.inline)
    }

    private func more(_ p: Playlist) {
        if !started { started = true; continuation = p.continuation }
        guard let c = continuation else { return }
        continuation = nil
        Task {
            if let page = try? await app.api?.playlistMore(c) {
                extra += page.tracks
                continuation = page.continuation
            }
        }
    }
}
#endif
