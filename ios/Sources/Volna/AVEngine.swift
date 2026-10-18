#if os(iOS)
import AVFoundation
import Foundation
import VolnaKit

/// AVPlayer as the core's platform player. It only plays what the core
/// loads (`http://127.0.0.1:<port>/…?codec=aac` from the core's audio
/// server) and tells `PlatformPlayerBridge` what happened; the bridge turns
/// that into reports. Main thread only.
final class AVEngine: NSObject, MediaEngine {
    weak var bridge: PlatformPlayerBridge?
    private let player = AVPlayer()
    private var token = 0
    private var startSeconds: Double = 0
    private var itemObservers: [NSKeyValueObservation] = []
    private var notes: [NSObjectProtocol] = []
    private var timeObserver: Any?
    private var statusObserver: NSKeyValueObservation?
    /// Set when the item is ready and the start seek is done.
    private var ready = false

    override init() {
        super.init()
        player.actionAtItemEnd = .pause
        player.automaticallyWaitsToMinimizeStalling = true
        statusObserver = player.observe(\.timeControlStatus, options: [.new]) { [weak self] p, _ in
            DispatchQueue.main.async { self?.timeControlChanged(p.timeControlStatus) }
        }
        timeObserver = player.addPeriodicTimeObserver(forInterval: CMTime(value: 1, timescale: 4), queue: .main) {
            [weak self] t in
            guard let self, self.ready else { return }
            self.bridge?.enginePosition(token: self.token, seconds: t.seconds)
        }
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(interrupted(_:)),
                           name: AVAudioSession.interruptionNotification, object: nil)
    }

    /// `.playback`: plays with the silent switch on and in the background
    /// (`UIBackgroundModes: audio`).
    static func activateSession() {
        let s = AVAudioSession.sharedInstance()
        do {
            try s.setCategory(.playback, mode: .default, policy: .longFormAudio)
            try s.setActive(true)
        } catch {
            NSLog("Volna: audio session: \(error)")
        }
    }

    // MARK: - MediaEngine

    func load(url: URL, startSeconds: Double, token: Int) {
        clearItem()
        self.token = token
        self.startSeconds = startSeconds
        ready = false
        Self.activateSession()
        let item = AVPlayerItem(url: url)
        item.preferredForwardBufferDuration = 30
        itemObservers.append(item.observe(\.status, options: [.new]) { [weak self] item, _ in
            DispatchQueue.main.async { self?.statusChanged(item, token: token) }
        })
        itemObservers.append(item.observe(\.duration, options: [.new]) { [weak self] item, _ in
            DispatchQueue.main.async {
                guard let self, self.token == token else { return }
                self.bridge?.engineDuration(token: token, duration: Self.seconds(item.duration))
            }
        })
        let center = NotificationCenter.default
        notes.append(center.addObserver(forName: AVPlayerItem.didPlayToEndTimeNotification, object: item,
                                        queue: .main) { [weak self] _ in
            self?.bridge?.engineEnded(token: token)
        })
        notes.append(center.addObserver(forName: AVPlayerItem.failedToPlayToEndTimeNotification, object: item,
                                        queue: .main) { [weak self] n in
            let err = n.userInfo?[AVPlayerItemFailedToPlayToEndTimeErrorKey] as? Error
            self?.bridge?.engineFailed(token: token, message: err?.localizedDescription ?? "playback failed")
        })
        player.replaceCurrentItem(with: item)
    }

    func play() {
        Self.activateSession()
        player.play()
    }

    func pause() { player.pause() }

    func seek(seconds: Double) {
        let t = CMTime(seconds: max(0, seconds), preferredTimescale: 1000)
        player.seek(to: t, toleranceBefore: .zero, toleranceAfter: .zero)
    }

    func stop() {
        player.pause()
        clearItem()
        player.replaceCurrentItem(with: nil)
    }

    func setVolume(_ v: Double) { player.volume = Float(v) }

    // MARK: - Observations

    private func clearItem() {
        itemObservers.forEach { $0.invalidate() }
        itemObservers = []
        notes.forEach { NotificationCenter.default.removeObserver($0) }
        notes = []
        ready = false
    }

    private static func seconds(_ t: CMTime) -> Double? {
        t.isNumeric && t.seconds.isFinite && t.seconds > 0 ? t.seconds : nil
    }

    private func statusChanged(_ item: AVPlayerItem, token: Int) {
        guard token == self.token, item === player.currentItem else { return }
        switch item.status {
        case .readyToPlay:
            guard !ready else { return }
            let done: (Bool) -> Void = { [weak self] _ in
                guard let self, self.token == token else { return }
                self.ready = true
                self.bridge?.engineReady(token: token, duration: Self.seconds(item.duration))
            }
            if startSeconds > 0.05 {
                item.seek(to: CMTime(seconds: startSeconds, preferredTimescale: 1000),
                          toleranceBefore: .zero, toleranceAfter: .zero) { ok in
                    DispatchQueue.main.async { done(ok) }
                }
            } else {
                done(true)
            }
        case .failed:
            let e = item.error as NSError?
            // AVFoundation decode errors: the bytes came but could not be played.
            let decode = e?.domain == AVFoundationErrorDomain
                && [AVError.decodeFailed.rawValue, AVError.fileFormatNotRecognized.rawValue].contains(e?.code ?? 0)
            bridge?.engineFailed(token: token, message: e?.localizedDescription ?? "cannot open the stream", decode: decode)
        default:
            break
        }
    }

    private func timeControlChanged(_ s: AVPlayer.TimeControlStatus) {
        guard ready else { return }
        switch s {
        case .playing:
            bridge?.enginePlaying(token: token)
        case .paused:
            bridge?.enginePaused(token: token)
        case .waitingToPlayAtSpecifiedRate:
            if player.reasonForWaitingToPlay == .toMinimizeStalls || player.reasonForWaitingToPlay == .evaluatingBufferingRate {
                bridge?.engineBuffering(token: token, true)
            }
        @unknown default:
            break
        }
    }

    /// A call or Siri took the audio; AVPlayer pauses by itself (reported as
    /// `paused`). When the system says so, resume through the core.
    @objc private func interrupted(_ n: Notification) {
        guard let info = n.userInfo,
              let raw = info[AVAudioSessionInterruptionTypeKey] as? UInt,
              AVAudioSession.InterruptionType(rawValue: raw) == .ended,
              let opts = info[AVAudioSessionInterruptionOptionKey] as? UInt,
              AVAudioSession.InterruptionOptions(rawValue: opts).contains(.shouldResume)
        else { return }
        DispatchQueue.main.async { [weak self] in self?.onResumeAfterInterruption?() }
    }

    /// Set by the app: ask the core to play again.
    var onResumeAfterInterruption: (() -> Void)?
}
#endif
