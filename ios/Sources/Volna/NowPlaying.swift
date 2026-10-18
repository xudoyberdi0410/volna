#if os(iOS)
import MediaPlayer
import UIKit
import VolnaKit

/// The lock screen, Control Center and headphone buttons: shows the player
/// model and sends the buttons to the core.
@MainActor
final class NowPlaying {
    private weak var app: AppModel?
    private var artworkFor: String?
    private var artwork: MPMediaItemArtwork?

    init(app: AppModel) {
        self.app = app
        let c = MPRemoteCommandCenter.shared()
        c.playCommand.addTarget { [weak self] _ in self?.send { try await $0.play() } ?? .commandFailed }
        c.pauseCommand.addTarget { [weak self] _ in self?.send { try await $0.pause() } ?? .commandFailed }
        c.togglePlayPauseCommand.addTarget { [weak self] _ in self?.send { try await $0.toggle() } ?? .commandFailed }
        c.nextTrackCommand.addTarget { [weak self] _ in self?.send { try await $0.next() } ?? .commandFailed }
        c.previousTrackCommand.addTarget { [weak self] _ in self?.send { try await $0.prev() } ?? .commandFailed }
        c.changePlaybackPositionCommand.addTarget { [weak self] e in
            guard let e = e as? MPChangePlaybackPositionCommandEvent else { return .commandFailed }
            return self?.send { try await $0.seek(e.positionTime) } ?? .commandFailed
        }
        c.likeCommand.localizedTitle = "Нравится"
        c.likeCommand.addTarget { [weak self] _ in
            self?.app?.likeCurrent()
            return .success
        }
        c.dislikeCommand.localizedTitle = "Не нравится"
        c.dislikeCommand.addTarget { [weak self] _ in
            self?.app?.dislikeCurrent()
            return .success
        }
        c.skipForwardCommand.isEnabled = false
        c.skipBackwardCommand.isEnabled = false
    }

    private func send(_ f: @escaping (API) async throws -> Void) -> MPRemoteCommandHandlerStatus {
        guard let app, app.api != nil else { return .commandFailed }
        app.run(f)
        return .success
    }

    /// Called on every player change.
    func update() {
        guard let app else { return }
        let center = MPNowPlayingInfoCenter.default()
        guard let t = app.player.current else {
            center.nowPlayingInfo = nil
            center.playbackState = .stopped
            return
        }
        let m = app.player
        var info: [String: Any] = [
            MPMediaItemPropertyTitle: t.title,
            MPMediaItemPropertyArtist: t.artistNames,
            MPNowPlayingInfoPropertyElapsedPlaybackTime: m.position(atMs: Date().unixMs),
            MPNowPlayingInfoPropertyPlaybackRate: m.isPlaying ? m.state.rate : 0,
            MPNowPlayingInfoPropertyDefaultPlaybackRate: 1.0,
            MPNowPlayingInfoPropertyMediaType: MPNowPlayingInfoMediaType.audio.rawValue,
        ]
        if let album = t.album?.title, !album.isEmpty { info[MPMediaItemPropertyAlbumTitle] = album }
        if let d = m.state.duration ?? t.duration { info[MPMediaItemPropertyPlaybackDuration] = d }
        if let cover = t.cover {
            if cover == artworkFor, let artwork {
                info[MPMediaItemPropertyArtwork] = artwork
            } else if cover != artworkFor {
                artworkFor = cover
                artwork = nil
                loadArtwork(cover)
            }
        }
        center.nowPlayingInfo = info
        center.playbackState = m.isPlaying ? .playing : .paused
        MPRemoteCommandCenter.shared().likeCommand.isActive = app.isLiked(t.trackId)
    }

    private func loadArtwork(_ cover: String) {
        guard let core = app?.core else { return }
        Task {
            guard let image = await ImageStore.shared.image(core: core, url: cover, size: 600) else { return }
            guard self.artworkFor == cover else { return }
            self.artwork = MPMediaItemArtwork(boundsSize: image.size) { _ in image }
            self.update()
        }
    }
}
#endif
