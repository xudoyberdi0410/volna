#if os(iOS)
import SwiftUI
import VolnaKit

/// Liked tracks, albums, artists, playlists; the user's playlists; history.
struct LibraryView: View {
    @Environment(AppModel.self) private var app
    @State private var kind: LikeKind = .track
    @State private var rows: [Liked] = []
    @State private var playlists: [PlaylistInfo] = []
    @State private var newName = ""
    @State private var creating = false

    var body: some View {
        List {
            Section {
                ForEach(playlists) { p in
                    NavigationLink(value: Route.userPlaylist(p.id)) {
                        HStack(spacing: 12) {
                            Cover(url: p.cover ?? p.thumbnails.first, size: 52, corner: 8)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(p.name).lineLimit(1)
                                Text("\(p.trackCount) треков").font(.subheadline).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                .onDelete { idx in
                    let ids = idx.map { playlists[$0].id }
                    app.run { api in for id in ids { _ = try await api.deletePlaylist(id) } }
                }
                Button { creating = true } label: { Label("Новый плейлист", systemImage: "plus") }
                NavigationLink(value: Route.history) { Label("История", systemImage: "clock") }
            } header: {
                Text("Мои плейлисты")
            }
            Section {
                Picker("Раздел", selection: $kind) {
                    ForEach(LikeKind.allCases, id: \.self) { Text($0.label).tag($0) }
                }
                .pickerStyle(.segmented)
                .listRowSeparator(.hidden)
                if rows.isEmpty {
                    Text(kind == .track ? "Здесь будут треки, которые вам понравились" : "Пока пусто")
                        .foregroundStyle(.secondary)
                }
                ForEach(rows) { row in
                    ItemRow(item: row.item, onPlay: playFrom(row.item))
                }
                .onDelete { idx in
                    let items = idx.map { rows[$0].item }
                    app.run { api in for item in items { _ = try await api.like(item, on: false) } }
                }
            } header: {
                Text("Любимое")
            }
        }
        .navigationTitle("Библиотека")
        .task(id: "\(kind.rawValue)-\(app.libraryRevision)") {
            rows = (try? await app.api?.library(kind)) ?? []
            playlists = (try? await app.api?.playlists()) ?? []
        }
        .alert("Новый плейлист", isPresented: $creating) {
            TextField("Название", text: $newName)
            Button("Создать") {
                let name = newName.trimmingCharacters(in: .whitespaces)
                newName = ""
                guard !name.isEmpty else { return }
                app.run { _ = try await $0.createPlaylist(name: name) }
            }
            Button("Отмена", role: .cancel) { newName = "" }
        }
    }

    private func playFrom(_ item: Item) -> (() -> Void)? {
        guard case .track(let t) = item else { return nil }
        return {
            let tracks = rows.compactMap { if case .track(let x) = $0.item { x } else { nil } }
            app.play(tracks, from: tracks.firstIndex(of: t) ?? 0)
        }
    }
}

struct UserPlaylistView: View {
    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    let id: String
    @State private var detail: PlaylistDetail?
    @State private var renaming = false
    @State private var name = ""

    var body: some View {
        Group {
            if let d = detail {
                let tracks = d.tracks.map(\.track)
                List {
                    Header(cover: d.info.cover ?? d.info.thumbnails.first, title: d.info.name,
                           subtitle: d.info.description.isEmpty ? "\(d.info.trackCount) треков" : d.info.description,
                           play: tracks.isEmpty ? nil : { app.play(tracks) },
                           shuffle: tracks.isEmpty ? nil : { app.play(tracks.shuffled()) })
                    ForEach(Array(d.tracks.enumerated()), id: \.offset) { i, e in
                        TrackRow(track: e.track) { app.play(tracks, from: i) }
                    }
                    .onDelete { idx in
                        let ids = idx.map { d.tracks[$0].track.videoId }
                        app.run { _ = try await $0.removeFromPlaylist(id, videoIds: ids) }
                    }
                    .onMove { from, to in
                        guard let f = from.first else { return }
                        app.run { try await $0.movePlaylistTrack(id, from: f, to: to > f ? to - 1 : to) }
                    }
                }
                .listStyle(.plain)
            } else {
                ProgressView()
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem {
                Menu {
                    Button { name = detail?.info.name ?? ""; renaming = true } label: { Label("Переименовать", systemImage: "pencil") }
                    EditButton()
                    Button(role: .destructive) {
                        app.run { _ = try await $0.deletePlaylist(id) }
                        dismiss()
                    } label: { Label("Удалить", systemImage: "trash") }
                } label: { Image(systemName: "ellipsis.circle") }
            }
        }
        .alert("Название", isPresented: $renaming) {
            TextField("Название", text: $name)
            Button("Сохранить") { app.run { _ = try await $0.renamePlaylist(id, name: name) } }
            Button("Отмена", role: .cancel) {}
        }
        .task(id: app.libraryRevision) { detail = try? await app.api?.userPlaylist(id) }
    }
}

struct HistoryView: View {
    @Environment(AppModel.self) private var app
    @State private var rows: [HistoryEntry] = []

    var body: some View {
        List {
            if rows.isEmpty { Text("Здесь появятся треки, которые вы слушали").foregroundStyle(.secondary) }
            ForEach(Array(rows.enumerated()), id: \.offset) { i, r in
                TrackRow(track: r.track) { app.play(rows.map(\.track), from: i) }
            }
        }
        .listStyle(.plain)
        .navigationTitle("История")
        .toolbar {
            ToolbarItem {
                Button("Очистить", role: .destructive) { app.run { try await $0.clearHistory() } }
                    .disabled(rows.isEmpty)
            }
        }
        .task(id: app.libraryRevision) { rows = (try? await app.api?.history(limit: 200)) ?? [] }
    }
}
#endif
