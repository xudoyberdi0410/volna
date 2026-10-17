#if os(iOS)
import AVKit
import SwiftUI
import VolnaKit

/// The bar above the tab bar.
struct MiniPlayer: View {
    @Environment(AppModel.self) private var app
    let open: () -> Void

    var body: some View {
        let t = app.player.current
        HStack(spacing: 12) {
            Cover(url: t?.cover, size: 36, corner: 6)
            VStack(alignment: .leading, spacing: 1) {
                Text(t?.title ?? "").font(.subheadline.weight(.semibold)).lineLimit(1)
                Text(t?.artistNames ?? "").font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 4)
            if app.player.isLoading {
                ProgressView().frame(width: 32)
            } else {
                Button { app.toggle() } label: {
                    Image(systemName: app.player.isPlaying ? "pause.fill" : "play.fill")
                        .font(.title3).frame(width: 32, height: 32)
                }
                .accessibilityLabel(app.player.isPlaying ? "Пауза" : "Играть")
            }
            Button { app.next() } label: {
                Image(systemName: "forward.fill").font(.title3).frame(width: 32, height: 32)
            }
            .accessibilityLabel("Следующий")
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 12)
        .contentShape(Rectangle())
        .onTapGesture(perform: open)
    }
}

/// Seconds as m:ss.
func clock(_ s: Double?) -> String {
    guard let s, s.isFinite, s >= 0 else { return "–:––" }
    let t = Int(s.rounded(.down))
    return t >= 3600 ? String(format: "%d:%02d:%02d", t / 3600, t / 60 % 60, t % 60) : String(format: "%d:%02d", t / 60, t % 60)
}

/// The full-screen player: cover, seek bar, controls, lyrics and queue.
struct FullPlayerView: View {
    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    @State private var page: Page = .cover
    @State private var scrub: Double?

    enum Page { case cover, lyrics, queue }

    var body: some View {
        let m = app.player
        let t = m.current
        ZStack {
            Backdrop(palette: app.palette)
            VStack(spacing: 20) {
                Capsule().fill(.secondary.opacity(0.5)).frame(width: 36, height: 5).padding(.top, 8)
                switch page {
                case .cover:
                    Spacer(minLength: 0)
                    LargeCover(url: t?.cover, corner: 18)
                        .shadow(color: .black.opacity(0.25), radius: 24, y: 12)
                        .scaleEffect(m.isPlaying ? 1 : 0.92)
                        .animation(.spring(response: 0.45, dampingFraction: 0.75), value: m.isPlaying)
                        .padding(.horizontal, 8)
                    Spacer(minLength: 0)
                case .lyrics:
                    LyricsView().frame(maxHeight: .infinity)
                case .queue:
                    QueueView().frame(maxHeight: .infinity)
                }
                titleRow(t)
                seekBar
                controls
                bottomRow
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 12)
        }
        .presentationDragIndicator(.hidden)
        .onChange(of: t == nil) { _, gone in if gone { dismiss() } }
    }

    private func titleRow(_ t: PlayerTrack?) -> some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text(t?.title ?? "").font(.title3.weight(.bold)).lineLimit(1)
                Text(t?.artistNames ?? "").font(.body).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            Button { app.likeCurrent() } label: {
                Image(systemName: app.isLiked(t?.trackId) ? "heart.fill" : "heart")
                    .font(.title2)
                    .symbolEffect(.bounce, value: app.isLiked(t?.trackId))
            }
            .accessibilityLabel("Нравится")
        }
    }

    private var seekBar: some View {
        TimelineView(.periodic(from: .now, by: 0.5)) { ctx in
            let m = app.player
            let duration = m.state.duration ?? m.current?.duration ?? 0
            let pos = scrub ?? m.position(atMs: ctx.date.unixMs)
            VStack(spacing: 4) {
                Slider(value: Binding(get: { min(pos, max(duration, 1)) }, set: { scrub = $0 }),
                       in: 0...max(duration, 1)) { editing in
                    if !editing, let s = scrub {
                        app.seek(s)
                        scrub = nil
                    }
                }
                .disabled(duration <= 0)
                HStack {
                    Text(clock(pos))
                    Spacer()
                    if let q = m.state.stream, let codec = q.codec {
                        Text([codec.uppercased(), q.bitrateKbps.map { "\(Int($0)) кбит/с" }].compactMap { $0 }.joined(separator: " · "))
                    }
                    Spacer()
                    Text("-" + clock(max(0, duration - pos)))
                }
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
            }
        }
    }

    private var controls: some View {
        let m = app.player
        return HStack {
            Button { app.run { try await $0.setShuffle(!m.shuffle) } } label: {
                Image(systemName: "shuffle").font(.body.weight(.semibold))
                    .foregroundStyle(m.shuffle ? Color.accentColor : .secondary)
            }
            .accessibilityLabel("Перемешать")
            Spacer()
            Button { app.prev() } label: { Image(systemName: "backward.fill").font(.title) }
                .accessibilityLabel("Предыдущий")
            Spacer()
            Button { app.toggle() } label: {
                ZStack {
                    if m.isLoading { ProgressView().controlSize(.large) } else {
                        Image(systemName: m.isPlaying ? "pause.fill" : "play.fill")
                            .font(.system(size: 44))
                            .contentTransition(.symbolEffect(.replace))
                    }
                }
                .frame(width: 64, height: 64)
            }
            .accessibilityLabel(m.isPlaying ? "Пауза" : "Играть")
            Spacer()
            Button { app.next() } label: { Image(systemName: "forward.fill").font(.title) }
                .accessibilityLabel("Следующий")
            Spacer()
            Button { app.run { try await $0.setRepeat(m.repeat.next) } } label: {
                Image(systemName: m.repeat == .one ? "repeat.1" : "repeat").font(.body.weight(.semibold))
                    .foregroundStyle(m.repeat == .off ? .secondary : Color.accentColor)
            }
            .accessibilityLabel("Повтор")
        }
        .buttonStyle(.plain)
    }

    private var bottomRow: some View {
        HStack {
            pageButton(.lyrics, "quote.bubble", "Текст")
            Spacer()
            if app.player.waveActive {
                Button { app.dislikeCurrent() } label: { Image(systemName: "hand.thumbsdown") }
                    .accessibilityLabel("Не нравится")
                Spacer()
            }
            RoutePicker().frame(width: 28, height: 28)
            Spacer()
            pageButton(.queue, "list.bullet", "Очередь")
        }
        .font(.title3)
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
    }

    private func pageButton(_ p: Page, _ icon: String, _ label: String) -> some View {
        Button {
            withAnimation(.snappy) { page = page == p ? .cover : p }
        } label: {
            Image(systemName: icon)
                .foregroundStyle(page == p ? Color.accentColor : .secondary)
        }
        .accessibilityLabel(label)
    }
}

/// The cover's colours, softly, behind the player.
struct Backdrop: View {
    let palette: Palette?

    var body: some View {
        let a = Color(hex: palette?.dominant) ?? Color(.systemBackground)
        let b = Color(hex: palette?.accent) ?? Color(.secondarySystemBackground)
        LinearGradient(colors: [a.opacity(0.85), b.opacity(0.55), Color(.systemBackground)],
                       startPoint: .top, endPoint: .bottom)
            .overlay(.ultraThinMaterial)
            .ignoresSafeArea()
            .animation(.easeInOut(duration: 0.6), value: palette)
    }
}

/// AirPlay / Bluetooth output picker.
struct RoutePicker: UIViewRepresentable {
    func makeUIView(context: Context) -> AVRoutePickerView {
        let v = AVRoutePickerView()
        v.tintColor = .secondaryLabel
        v.prioritizesVideoDevices = false
        return v
    }

    func updateUIView(_ uiView: AVRoutePickerView, context: Context) {}
}

/// Up next, reorderable.
struct QueueView: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        let m = app.player
        List {
            if let t = m.current {
                Section("Сейчас играет") { QueueRow(track: t, current: true) }
            }
            Section {
                ForEach(m.upNext) { t in
                    QueueRow(track: t, current: false)
                        .contentShape(Rectangle())
                        .onTapGesture { app.run { try await $0.playUid(t.uid) } }
                }
                .onDelete { idx in
                    let uids = idx.map { m.upNext[$0].uid }
                    app.run { api in for uid in uids { try await api.remove(uid: uid) } }
                }
                .onMove { from, to in
                    guard let f = from.first else { return }
                    let uid = m.upNext[f].uid
                    // Positions in the whole order: after the current entry.
                    let target = m.index + 1 + (to > f ? to - 1 : to)
                    app.run { try await $0.move(uid: uid, to: target) }
                }
            } header: {
                HStack {
                    Text("Далее")
                    Spacer()
                    if m.continuation.fetching { ProgressView().controlSize(.small) }
                    if m.waveActive { Text("Моя волна").foregroundStyle(.secondary) }
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .environment(\.editMode, .constant(.active))
    }
}

struct QueueRow: View {
    let track: PlayerTrack
    let current: Bool

    var body: some View {
        HStack(spacing: 12) {
            Cover(url: track.cover, size: 40, corner: 6)
            VStack(alignment: .leading, spacing: 2) {
                Text(track.title).font(.subheadline.weight(current ? .bold : .regular)).lineLimit(1)
                Text(track.artistNames).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            if track.unavailable { Image(systemName: "exclamationmark.circle").foregroundStyle(.secondary) }
        }
        .listRowBackground(Color.clear)
    }
}

/// Synced lyrics: the current line lit, tap a line to go there.
struct LyricsView: View {
    @Environment(AppModel.self) private var app
    @State private var lyrics: Lyrics?
    @State private var loading = false
    @State private var failed = false

    var body: some View {
        let t = app.player.current
        Group {
            if loading {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let l = lyrics, l.synced != .none {
                synced(l)
            } else if let l = lyrics, let plain = l.plain ?? Optional(l.lines.map(\.text).joined(separator: "\n")), !plain.isEmpty {
                ScrollView {
                    Text(plain).font(.title3.weight(.semibold)).frame(maxWidth: .infinity, alignment: .leading)
                    Text("Источник: \(l.source)").font(.caption).foregroundStyle(.secondary).padding(.top, 12)
                }
            } else {
                ContentUnavailableView(failed ? "Не удалось загрузить текст" : "Текста нет",
                                       systemImage: "quote.bubble")
            }
        }
        .task(id: t?.trackId) {
            guard let t, let api = app.api else { return }
            loading = true
            failed = false
            do {
                lyrics = try await api.lyrics(videoId: t.trackId, title: t.title, artist: t.artistNames,
                                              album: t.album?.title, durationMs: t.duration.map { $0 * 1000 })
            } catch {
                lyrics = nil
                failed = true
            }
            loading = false
        }
    }

    private func synced(_ l: Lyrics) -> some View {
        TimelineView(.periodic(from: .now, by: 0.25)) { ctx in
            let pos = app.player.position(atMs: ctx.date.unixMs) * 1000
            let current = l.lineIndex(at: pos)
            ScrollViewReader { proxy in
                ScrollView(showsIndicators: false) {
                    VStack(alignment: .leading, spacing: 18) {
                        ForEach(Array(l.lines.enumerated()), id: \.offset) { i, line in
                            Text(line.text.isEmpty ? "♪" : line.text)
                                .font(.title2.weight(.bold))
                                .foregroundStyle(i == current ? .primary : .secondary)
                                .opacity(i == current ? 1 : 0.55)
                                .scaleEffect(i == current ? 1.0 : 0.97, anchor: .leading)
                                .animation(.easeOut(duration: 0.25), value: current)
                                .id(i)
                                .onTapGesture { app.seek(line.startMs / 1000) }
                        }
                        Text("Источник: \(l.source)").font(.caption).foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 40)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .onChange(of: current) { _, c in
                    guard let c else { return }
                    withAnimation(.easeInOut(duration: 0.4)) { proxy.scrollTo(c, anchor: .center) }
                }
            }
        }
    }
}
#endif
