package uz.khudoberdi.volna.core

import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.util.Log
import org.json.JSONObject
import uz.khudoberdi.volna.BuildConfig
import uz.khudoberdi.volna.PlaybackService
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/**
 * The process's `volna-core`. Started by whoever comes first (the Flutter
 * plugin or [PlaybackService]); the Flutter UI attaches to the same handle
 * over `dart:ffi`, the service drives the platform player and the WebView
 * JS engine. Because the core belongs to the process and the service, not
 * to the Flutter isolate, playback goes on with no activity.
 */
object VolnaCore : VolnaCoreNative.Host {
    private const val TAG = "VolnaCore"

    /** The platform player the core commands (the media service). */
    interface Player {
        /** On the main thread. */
        fun onCoreCommand(command: JSONObject)
    }

    private val main = Handler(Looper.getMainLooper())
    private lateinit var appContext: Context

    @Volatile
    var handle: Long = 0
        private set

    private var player: Player? = null
    /** Commands that came before the service was up; replayed in order. */
    private val pending = ArrayDeque<JSONObject>()
    private var js: JsEngine? = null

    private val nextToken = AtomicLong(1)
    private val calls = ConcurrentHashMap<Long, (Boolean, String) -> Unit>()
    private val protos = ConcurrentHashMap<Long, (Int, ByteArray) -> Unit>()

    /** Starts the core if needed and returns its handle. Throws if it cannot start. */
    @Synchronized
    fun start(context: Context): Long {
        if (handle != 0L) return handle
        appContext = context.applicationContext
        // Same places path_provider reports: getApplicationSupportDirectory()
        // is filesDir; the core's covers stay apart from the Dart image cache.
        val config = JSONObject()
            .put("dataDir", appContext.filesDir.absolutePath)
            .put("cacheDir", File(appContext.cacheDir, "core").absolutePath)
            .put("audioServer", true)
        importDebugCookies()
        cookiesFile().takeIf { it.isFile }?.readText()?.let { config.put("youtubeCookies", it) }
        // Debug builds only (the core ignores both in release builds too).
        if (BuildConfig.DEBUG) {
            if (BuildConfig.DEV_PROXY.isNotEmpty()) config.put("devProxy", BuildConfig.DEV_PROXY)
            if (BuildConfig.DEV_STREAM_URL.isNotEmpty()) {
                config.put("devStreamUrl", BuildConfig.DEV_STREAM_URL)
            }
        }
        handle = VolnaCoreNative.nativeStart(config.toString(), this)
        Log.i(TAG, "core started")
        return handle
    }

    private fun cookiesFile() = File(appContext.filesDir, "youtube_cookies.txt")

    /**
     * Debug builds: a cookies.txt pushed to the app's external files dir
     * (`adb push cookies.txt /sdcard/Android/data/uz.khudoberdi.volna/files/youtube_cookies.txt`)
     * is moved in as the YouTube session, for emulator tests without a
     * Google sign-in.
     */
    private fun importDebugCookies() {
        if (!BuildConfig.DEBUG) return
        val pushed = appContext.getExternalFilesDir(null)?.let { File(it, "youtube_cookies.txt") }
        if (pushed == null || !pushed.isFile) return
        pushed.copyTo(cookiesFile(), overwrite = true)
        pushed.delete()
        Log.w(TAG, "debug: imported a pushed YouTube cookies.txt")
    }

    fun hasYoutubeCookies(): Boolean = ::appContext.isInitialized && cookiesFile().isFile

    /**
     * Saves the YouTube session (Netscape cookies.txt; null signs out) and
     * hands it to the running core with `youtube_set_cookies`. [done] gets
     * false when this core cannot take it at runtime: it applies from the
     * next start (init config `youtubeCookies`).
     */
    fun setYoutubeCookies(cookies: String?, done: (Boolean) -> Unit) {
        val file = cookiesFile()
        if (cookies == null) file.delete() else file.writeText(cookies)
        call("youtube_set_cookies", JSONObject().put("cookies", cookies ?: JSONObject.NULL)) { ok, json ->
            if (!ok) Log.w(TAG, "youtube_set_cookies: $json")
            done(ok)
        }
    }

    /** The media service is ready for commands (main thread). */
    fun attachPlayer(p: Player) {
        player = p
        while (pending.isNotEmpty()) p.onCoreCommand(pending.removeFirst())
    }

    fun detachPlayer(p: Player) {
        if (player === p) player = null
    }

    /** The Flutter engine is going away: its FFI callbacks must not fire anymore. */
    fun detachUi() {
        if (handle != 0L) VolnaCoreNative.nativeDetachUi(handle)
    }

    /** Runs a core command; [done] gets (ok, JSON) on the main thread. */
    fun call(cmd: String, args: JSONObject = JSONObject(), done: (Boolean, String) -> Unit = { _, _ -> }) {
        if (handle == 0L) {
            main.post { done(false, "\"unavailable: the core is not running\"") }
            return
        }
        val token = nextToken.getAndIncrement()
        calls[token] = done
        VolnaCoreNative.nativeCall(handle, token, cmd, args.toString())
    }

    /** A `volna://` GET; [done] gets (status, body) on the main thread. */
    fun proto(uri: String, done: (Int, ByteArray) -> Unit) {
        if (handle == 0L) return
        val token = nextToken.getAndIncrement()
        protos[token] = done
        VolnaCoreNative.nativeProto(handle, token, uri)
    }

    /** What the platform player did (`PlayerReport` JSON). */
    fun report(json: JSONObject) {
        if (handle != 0L) VolnaCoreNative.nativePlayerReport(handle, json.toString())
    }

    fun jsResult(id: Long, ok: Boolean, json: String) {
        if (handle != 0L) VolnaCoreNative.nativeJsResult(handle, id, ok, json)
    }

    // VolnaCoreNative.Host: core threads.

    override fun onPlayerCommand(json: String) {
        val command = try {
            JSONObject(json)
        } catch (e: Exception) {
            Log.e(TAG, "bad player command $json", e)
            return
        }
        main.post {
            val p = player
            if (p != null) {
                p.onCoreCommand(command)
            } else {
                pending.addLast(command)
                startService()
            }
        }
    }

    override fun onJsRequest(id: Long, json: String) {
        main.post {
            val engine = js ?: JsEngine(appContext).also { js = it }
            engine.run(id, json)
        }
    }

    override fun onCallResult(token: Long, ok: Boolean, json: String) {
        val done = calls.remove(token) ?: return
        main.post { done(ok, json) }
    }

    override fun onProtoResult(token: Long, status: Int, body: ByteArray) {
        val done = protos.remove(token) ?: return
        main.post { done(status, body) }
    }

    /** Brings the media service up (it attaches itself as the player). */
    fun startService() {
        if (!::appContext.isInitialized) return
        try {
            appContext.startService(Intent(appContext, PlaybackService::class.java))
        } catch (e: IllegalStateException) {
            // In the background without playback the system refuses; the
            // commands wait until the app is in front again.
            Log.w(TAG, "cannot start the playback service now: ${e.message}")
        }
    }
}
