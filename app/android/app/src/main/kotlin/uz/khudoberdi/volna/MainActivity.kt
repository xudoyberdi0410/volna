package uz.khudoberdi.volna

import android.content.Intent
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import uz.khudoberdi.volna.core.VolnaCore

/**
 * The Flutter UI. It does not own the core: `volna/core` `handle` starts
 * the process's core (or returns it) and the media service, and Dart
 * attaches to that handle over `dart:ffi` (`lib/ipc/ffi_transport.dart`).
 * When the engine goes, its FFI callbacks are dropped and playback goes on
 * in [PlaybackService].
 */
class MainActivity : FlutterActivity() {
    private var signIn: MethodChannel.Result? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "volna/core")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "handle" -> try {
                        val handle = VolnaCore.start(this)
                        VolnaCore.startService()
                        result.success(handle)
                    } catch (e: Exception) {
                        result.error("core", e.message, null)
                    }
                    "googleSignIn" -> {
                        signIn?.success(null)
                        signIn = result
                        @Suppress("DEPRECATION")
                        startActivityForResult(Intent(this, GoogleSignInActivity::class.java), SIGN_IN)
                    }
                    "googleSignOut" -> VolnaCore.setYoutubeCookies(null) { applied ->
                        result.success(mapOf("applied" to applied))
                    }
                    "googleSignedIn" -> result.success(VolnaCore.hasYoutubeCookies())
                    else -> result.notImplemented()
                }
            }
    }

    override fun cleanUpFlutterEngine(flutterEngine: FlutterEngine) {
        VolnaCore.detachUi()
        super.cleanUpFlutterEngine(flutterEngine)
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != SIGN_IN) return
        val result = signIn ?: return
        signIn = null
        val cookies = data?.getStringExtra(GoogleSignInActivity.EXTRA_COOKIES)
        if (resultCode != RESULT_OK || cookies == null) {
            result.success(null)
            return
        }
        VolnaCore.setYoutubeCookies(cookies) { applied ->
            // applied=false: saved, used from the next start of the app.
            result.success(mapOf("applied" to applied))
        }
    }

    companion object {
        private const val SIGN_IN = 7001
    }
}
