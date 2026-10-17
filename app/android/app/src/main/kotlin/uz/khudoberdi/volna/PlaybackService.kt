package uz.khudoberdi.volna

import android.app.PendingIntent
import android.content.Intent
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.annotation.OptIn
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import org.json.JSONObject
import uz.khudoberdi.volna.core.VolnaCore

/**
 * The platform player of `volna-core` (the PlatformPlayer contract in
 * `crates/core/src/player/platform.rs`): a Media3 [MediaSessionService]
 * with ExoPlayer, a foreground service with the media notification,
 * headset buttons and audio focus.
 *
 * The core owns the queue and every decision. Core commands (load, play,
 * pause, seek, stop, setVolume) drive ExoPlayer directly; ExoPlayer's
 * state goes back as reports (ready, playing, paused, position, buffering,
 * ended, error). What the user does in the notification, on a headset or in
 * Android Auto goes through [CorePlayer] to the core's commands instead, so
 * the core stays the one source of truth. The loaded URL is always the
 * core's loopback audio server.
 */
@OptIn(UnstableApi::class)
class PlaybackService : MediaSessionService(), VolnaCore.Player {
    private lateinit var exo: ExoPlayer
    private var session: MediaSession? = null
    private val main = Handler(Looper.getMainLooper())

    /** The item of the latest load; reports are about it only. */
    private var loaded = false
    private var readySent = false
    private var buffering = false
    private var loadedUid: String? = null

    private val ticker = object : Runnable {
        override fun run() {
            if (loaded && exo.isPlaying) {
                reportPosition()
                main.postDelayed(this, POSITION_EVERY_MS)
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        VolnaCore.start(this)
        exo = ExoPlayer.Builder(this)
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(C.USAGE_MEDIA)
                    .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
                    .build(),
                /* handleAudioFocus = */ true,
            )
            .setHandleAudioBecomingNoisy(true)
            .setWakeMode(C.WAKE_MODE_NETWORK)
            .build()
        exo.addListener(listener)
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        session = MediaSession.Builder(this, CorePlayer(exo))
            .setSessionActivity(open)
            .build()
        VolnaCore.attachPlayer(this)
        Log.i(TAG, "playback service up")
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    override fun onTaskRemoved(rootIntent: Intent?) {
        // Swiped away while paused: nothing to keep alive for.
        if (!exo.playWhenReady || exo.mediaItemCount == 0) stopSelf()
    }

    override fun onDestroy() {
        VolnaCore.detachPlayer(this)
        main.removeCallbacks(ticker)
        session?.release()
        session = null
        exo.release()
        super.onDestroy()
    }

    // Core → ExoPlayer (main thread).

    override fun onCoreCommand(command: JSONObject) {
        when (command.optString("op")) {
            "load" -> load(command.getString("url"), command.optLong("startMs", 0))
            "play" -> exo.play()
            "pause" -> exo.pause()
            "seek" -> exo.seekTo(command.optLong("positionMs", 0))
            "stop" -> {
                loaded = false
                readySent = false
                main.removeCallbacks(ticker)
                exo.stop()
                exo.clearMediaItems()
            }
            "setVolume" -> exo.volume = command.optDouble("volume", 1.0).toFloat().coerceIn(0f, 1f)
            else -> Log.w(TAG, "unknown player command $command")
        }
    }

    private fun load(url: String, startMs: Long) {
        loaded = true
        readySent = false
        buffering = false
        loadedUid = null
        val item = MediaItem.Builder().setUri(Uri.parse(url)).setMediaId(url).build()
        exo.playWhenReady = false
        exo.setMediaItem(item, startMs)
        exo.prepare()
        updateMetadata()
    }

    /** Title, artist and cover of the current queue entry, for the notification. */
    private fun updateMetadata() {
        VolnaCore.call("player_snapshot") { ok, json ->
            if (!ok || !loaded || exo.mediaItemCount == 0) return@call
            val snap = try {
                JSONObject(json)
            } catch (e: Exception) {
                return@call
            }
            val uid = snap.optJSONObject("state")?.optString("uid")?.takeIf { it.isNotEmpty() }
                ?: return@call
            if (uid == loadedUid) return@call
            val items = snap.optJSONObject("queue")?.optJSONArray("items") ?: return@call
            val track = (0 until items.length())
                .map { items.getJSONObject(it) }
                .firstOrNull { it.optString("uid") == uid } ?: return@call
            loadedUid = uid
            val artists = track.optJSONArray("artists")
            val artist = (0 until (artists?.length() ?: 0))
                .mapNotNull { artists?.getJSONObject(it)?.optString("name")?.takeIf { n -> n.isNotEmpty() } }
                .joinToString(", ")
            val meta = MediaMetadata.Builder()
                .setTitle(track.optString("title"))
                .setArtist(artist)
                .setAlbumTitle(track.optJSONObject("album")?.optString("title"))
                .build()
            setMetadata(meta)
            val cover = track.optString("cover").takeIf { it.isNotEmpty() } ?: return@call
            // Through the core like every image (network settings apply).
            val uri = if (cover.startsWith("volna://")) cover
            else "volna://img/?u=${Uri.encode(cover)}&s=544"
            VolnaCore.proto(uri) { status, body ->
                if (status in 200..299 && body.isNotEmpty() && loadedUid == uid) {
                    setMetadata(
                        meta.buildUpon()
                            .setArtworkData(body, MediaMetadata.PICTURE_TYPE_FRONT_COVER)
                            .build(),
                    )
                }
            }
        }
    }

    private fun setMetadata(meta: MediaMetadata) {
        if (exo.mediaItemCount == 0) return
        val item = exo.getMediaItemAt(0)
        // Same URI: ExoPlayer updates the metadata without reloading.
        exo.replaceMediaItem(0, item.buildUpon().setMediaMetadata(meta).build())
    }

    // ExoPlayer → core reports.

    private fun report(type: String, fill: JSONObject.() -> Unit = {}) {
        VolnaCore.report(JSONObject().put("type", type).apply(fill))
    }

    private fun durationMs(): Long? = exo.duration.takeIf { it != C.TIME_UNSET && it > 0 }

    private fun reportPosition() {
        report("position") {
            put("positionMs", exo.currentPosition.coerceAtLeast(0))
            durationMs()?.let { put("durationMs", it) }
        }
    }

    private val listener = object : Player.Listener {
        override fun onPlaybackStateChanged(state: Int) {
            if (!loaded) return
            when (state) {
                Player.STATE_READY -> {
                    if (!readySent) {
                        readySent = true
                        report("ready") { durationMs()?.let { put("durationMs", it) } }
                        if (loadedUid == null) updateMetadata()
                    } else if (buffering) {
                        buffering = false
                        report("buffering") { put("on", false) }
                    }
                }
                Player.STATE_BUFFERING -> if (readySent && !buffering) {
                    buffering = true
                    report("buffering") { put("on", true) }
                }
                Player.STATE_ENDED -> {
                    main.removeCallbacks(ticker)
                    report("ended")
                }
                Player.STATE_IDLE -> Unit
            }
        }

        override fun onIsPlayingChanged(isPlaying: Boolean) {
            if (!loaded) return
            if (isPlaying) {
                report("playing")
                reportPosition()
                main.removeCallbacks(ticker)
                main.postDelayed(ticker, POSITION_EVERY_MS)
            } else {
                main.removeCallbacks(ticker)
                reportPosition()
                if (exo.playbackState != Player.STATE_ENDED) report("paused")
            }
        }

        override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
            // ExoPlayer paused on its own (headphones out, focus taken by
            // another app): the core must know, or it would wait forever.
            if (!playWhenReady && loaded && (
                    reason == Player.PLAY_WHEN_READY_CHANGE_REASON_AUDIO_BECOMING_NOISY ||
                        reason == Player.PLAY_WHEN_READY_CHANGE_REASON_AUDIO_FOCUS_LOSS
                    )
            ) {
                VolnaCore.call("player_pause")
            }
        }

        override fun onPlayerError(error: PlaybackException) {
            if (!loaded) return
            main.removeCallbacks(ticker)
            Log.w(TAG, "playback error ${error.errorCodeName}", error)
            val decode = error.errorCode in 4000..4999
            report("error") {
                put("message", "${error.errorCodeName}: ${error.message}")
                put("decode", decode)
            }
        }
    }

    /**
     * What the session (notification, headset, Android Auto, other apps)
     * sees. User actions become core commands; the core then commands
     * ExoPlayer. Next/previous are always offered: the core knows the queue.
     */
    private class CorePlayer(player: ExoPlayer) : ForwardingPlayer(player) {
        override fun play() = VolnaCore.call("player_play")
        override fun pause() = VolnaCore.call("player_pause")
        override fun setPlayWhenReady(playWhenReady: Boolean) =
            if (playWhenReady) play() else pause()

        override fun stop() = VolnaCore.call("player_pause")
        override fun seekToNext() = VolnaCore.call("player_next")
        override fun seekToNextMediaItem() = seekToNext()
        override fun seekToPrevious() = VolnaCore.call("player_prev")
        override fun seekToPreviousMediaItem() = seekToPrevious()
        override fun seekTo(positionMs: Long) =
            VolnaCore.call("player_seek", JSONObject().put("position", positionMs / 1000.0))

        override fun getAvailableCommands(): Player.Commands =
            super.getAvailableCommands().buildUpon()
                .addAll(
                    Player.COMMAND_PLAY_PAUSE,
                    Player.COMMAND_SEEK_TO_NEXT,
                    Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                    Player.COMMAND_SEEK_TO_PREVIOUS,
                    Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM,
                )
                .build()

        override fun isCommandAvailable(command: Int): Boolean =
            availableCommands.contains(command)
    }

    companion object {
        private const val TAG = "VolnaPlayback"
        private const val POSITION_EVERY_MS = 500L
    }
}
