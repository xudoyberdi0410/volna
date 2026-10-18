#if os(iOS)
import SwiftUI
import VolnaKit

/// «Моя волна»: one big button, then the cover with like / dislike / skip.
struct WaveView: View {
    @Environment(AppModel.self) private var app
    @State private var cold = false
    @State private var settings = false

    var body: some View {
        let m = app.player
        ZStack {
            Backdrop(palette: m.waveActive ? app.palette : nil)
            if m.waveActive, let t = m.current {
                playing(t)
            } else {
                idle
            }
        }
        .navigationTitle("Моя волна")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button { settings = true } label: { Image(systemName: "slider.horizontal.3") }
                    .accessibilityLabel("Настроить волну")
            }
        }
        .sheet(isPresented: $settings) { NavigationStack { WaveSettingsView() } }
        .sheet(isPresented: $cold) { NavigationStack { ColdStartView { cold = false; app.startWave() } } }
        .task(id: app.libraryRevision) {
            if let s = try? await app.api?.waveStatus() { cold = s.cold && !m.waveActive }
        }
    }

    private var idle: some View {
        VStack(spacing: 28) {
            Spacer()
            Button {
                Task {
                    if let s = try? await app.api?.waveStatus(), s.cold { cold = true } else { app.startWave() }
                }
            } label: {
                ZStack {
                    Circle().fill(.tint.opacity(0.15)).frame(width: 220, height: 220)
                    Circle().fill(.tint).frame(width: 150, height: 150)
                        .shadow(color: .accentColor.opacity(0.4), radius: 24, y: 10)
                    Image(systemName: "play.fill").font(.system(size: 52)).foregroundStyle(.white)
                }
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Запустить волну")
            VStack(spacing: 6) {
                Text("Моя волна").font(.largeTitle.weight(.bold))
                Text("Бесконечный поток под ваш вкус").foregroundStyle(.secondary)
            }
            if !app.googleSignedIn {
                NavigationLink {
                    GoogleSignInView()
                } label: {
                    Label("Войдите в Google, чтобы звук воспроизводился", systemImage: "person.crop.circle.badge.exclamationmark")
                        .font(.footnote)
                }
            }
            Spacer()
            Spacer()
        }
        .padding()
    }

    private func playing(_ t: PlayerTrack) -> some View {
        let m = app.player
        return VStack(spacing: 24) {
            Spacer(minLength: 8)
            LargeCover(url: t.cover, corner: 22)
                .shadow(color: .black.opacity(0.25), radius: 30, y: 14)
                .padding(.horizontal, 32)
                .id(t.uid)
                .transition(.asymmetric(insertion: .move(edge: .trailing).combined(with: .opacity),
                                        removal: .move(edge: .leading).combined(with: .opacity)))
            VStack(spacing: 6) {
                Text(t.title).font(.title2.weight(.bold)).lineLimit(2).multilineTextAlignment(.center)
                Text(t.artistNames).font(.body).foregroundStyle(.secondary).lineLimit(1)
            }
            .padding(.horizontal)
            HStack(spacing: 36) {
                round("hand.thumbsdown", "Не нравится") { app.dislikeCurrent() }
                Button { app.toggle() } label: {
                    Image(systemName: m.isPlaying ? "pause.fill" : "play.fill")
                        .font(.system(size: 34))
                        .contentTransition(.symbolEffect(.replace))
                        .frame(width: 76, height: 76)
                        .background(.tint, in: Circle())
                        .foregroundStyle(.white)
                }
                .accessibilityLabel(m.isPlaying ? "Пауза" : "Играть")
                round(app.isLiked(t.trackId) ? "heart.fill" : "heart", "Нравится") { app.likeCurrent() }
            }
            .buttonStyle(.plain)
            HStack(spacing: 28) {
                Button { app.stopWave() } label: { Label("Стоп", systemImage: "stop.fill") }
                Button { app.next() } label: { Label("Дальше", systemImage: "forward.fill") }
            }
            .font(.subheadline.weight(.medium))
            .buttonStyle(.bordered)
            .buttonBorderShape(.capsule)
            Spacer(minLength: 8)
        }
        .animation(.spring(response: 0.5, dampingFraction: 0.85), value: t.uid)
    }

    private func round(_ icon: String, _ label: String, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: icon).font(.title2).frame(width: 56, height: 56)
                .background(.thinMaterial, in: Circle())
        }
        .accessibilityLabel(label)
    }
}

/// The first start: pick a few artists to seed the wave.
struct ColdStartView: View {
    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    let done: () -> Void
    @State private var query = ""
    @State private var artists: [ArtistCard] = []
    @State private var picked: [ArtistCard] = []

    var body: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 100), spacing: 16)], spacing: 18) {
                ForEach(artists) { a in
                    let on = picked.contains(a)
                    Button {
                        if on { picked.removeAll { $0 == a } } else { picked.append(a) }
                    } label: {
                        VStack(spacing: 6) {
                            Cover(url: a.picture, size: 96, round: true)
                                .overlay(alignment: .bottomTrailing) {
                                    if on {
                                        Image(systemName: "checkmark.circle.fill").font(.title2)
                                            .foregroundStyle(.white, Color.accentColor)
                                    }
                                }
                                .scaleEffect(on ? 0.94 : 1)
                            Text(a.name).font(.footnote).lineLimit(1)
                        }
                    }
                    .buttonStyle(.plain)
                    .animation(.snappy, value: on)
                }
            }
            .padding()
        }
        .searchable(text: $query, prompt: "Исполнитель")
        .task(id: query) {
            try? await Task.sleep(nanoseconds: 300_000_000)
            artists = (try? await app.api?.waveArtistSearch(query)) ?? artists
        }
        .navigationTitle("Кого вы слушаете?")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Отмена") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                Button("Готово (\(picked.count))") {
                    let chosen = picked
                    Task {
                        _ = try? await app.api?.waveColdStart(chosen)
                        done()
                    }
                }
                .disabled(picked.count < 1)
            }
        }
    }
}

/// Character, mood, activity, language.
struct WaveSettingsView: View {
    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    @State private var s: WaveSettings?
    @State private var activities: WaveActivities?

    var body: some View {
        Form {
            if let current = s {
                Section("Характер") {
                    Picker("Характер", selection: binding(\.character, current)) {
                        ForEach(WaveSettings.characters, id: \.0) { Text($0.1).tag($0.0) }
                    }
                    .pickerStyle(.segmented)
                }
                Section("Настроение") {
                    Picker("Настроение", selection: binding(\.mood, current)) {
                        Text("Любое").tag(String?.none)
                        ForEach(WaveSettings.moods, id: \.0) { Text($0.1).tag(String?.some($0.0)) }
                    }
                }
                Section("Язык") {
                    Picker("Язык", selection: binding(\.language, current)) {
                        ForEach(WaveSettings.languages, id: \.0) { Text($0.1).tag($0.0) }
                    }
                }
                if let a = activities {
                    Section("Занятие или жанр") {
                        Picker("Занятие", selection: binding(\.activity, current)) {
                            Text("Не важно").tag(WaveActivity?.none)
                            ForEach(a.activities + a.genres) { x in
                                Text(x.label ?? x.title).tag(WaveActivity?.some(WaveActivity(title: x.title, params: x.params)))
                            }
                        }
                        .pickerStyle(.navigationLink)
                    }
                }
            } else {
                ProgressView()
            }
        }
        .navigationTitle("Настройки волны")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Готово") { dismiss() } } }
        .task {
            s = try? await app.api?.waveStatus().settings
            activities = try? await app.api?.waveActivities()
        }
    }

    /// Edits one field and saves the settings (the wave re-tunes itself).
    private func binding<V>(_ key: WritableKeyPath<WaveSettings, V>, _ current: WaveSettings) -> Binding<V> {
        Binding(get: { (s ?? current)[keyPath: key] }, set: { v in
            var next = s ?? current
            next[keyPath: key] = v
            s = next
            app.run { try await $0.setWaveSettings(next) }
        })
    }
}
#endif
