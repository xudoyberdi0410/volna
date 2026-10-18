package uz.khudoberdi.volna.core

import android.annotation.SuppressLint
import android.content.Context
import android.util.Log
import android.webkit.JavascriptInterface
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebView
import android.webkit.WebViewClient

/**
 * The JavaScript engine of the stream resolver (docs/streams.md, "JsRuntime
 * contract"): one long-lived headless WebView with `volna_core_js_prelude()`
 * loaded; each job evaluates `volnaJs(<request JSON>)` and hands the string
 * the promise resolves to back to the core unchanged.
 *
 * The page never touches the network (`blockNetworkLoads`): everything it
 * needs comes from Rust, which applies the network settings. If the page
 * dies it is rebuilt; the core resends what was lost (`need_player` /
 * `unknown_session`). Main thread only.
 */
@SuppressLint("SetJavaScriptEnabled", "JavascriptInterface")
internal class JsEngine(private val context: Context) {
    private var web: WebView? = null
    private var ready = false
    private val queue = ArrayDeque<Pair<Long, String>>()

    /** Called by the page from the JavaBridge thread. */
    private class Bridge {
        @JavascriptInterface
        fun reply(id: Long, result: String?) {
            VolnaCore.jsResult(id, true, result ?: "null")
        }

        @JavascriptInterface
        fun fail(id: Long, message: String?) {
            VolnaCore.jsResult(id, false, message ?: "error")
        }
    }

    fun run(id: Long, request: String) {
        if (web == null) create()
        if (!ready) {
            queue.addLast(id to request)
            return
        }
        eval(id, request)
    }

    private fun eval(id: Long, request: String) {
        // JSON is a JS expression; `volnaJs` never rejects, the catch is for
        // a missing prelude.
        val script = "volnaJs($request).then(" +
            "s => VolnaBridge.reply($id, s), e => VolnaBridge.fail($id, String(e)));void 0"
        web?.evaluateJavascript(script, null)
    }

    private fun create() {
        ready = false
        val w = WebView(context.applicationContext)
        w.settings.javaScriptEnabled = true
        w.settings.blockNetworkLoads = true
        w.settings.allowFileAccess = false
        w.settings.allowContentAccess = false
        w.addJavascriptInterface(Bridge(), "VolnaBridge")
        w.webViewClient = object : WebViewClient() {
            override fun onPageFinished(view: WebView, url: String?) {
                if (ready || view !== web) return
                val prelude = VolnaCoreNative.nativeJsPrelude()
                view.evaluateJavascript("$prelude\n;typeof volnaJs") { type ->
                    Log.i(TAG, "prelude loaded (${prelude.length} chars), volnaJs: $type")
                    ready = true
                    while (queue.isNotEmpty()) {
                        val (id, req) = queue.removeFirst()
                        eval(id, req)
                    }
                }
            }

            override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
                Log.w(TAG, "JS page died (crash: ${detail.didCrash()}); rebuilding on the next job")
                if (view === web) {
                    web = null
                    ready = false
                }
                view.destroy()
                return true
            }
        }
        web = w
        // A real document for BotGuard; nothing is fetched.
        w.loadDataWithBaseURL(
            "https://music.youtube.com/",
            "<!doctype html><html><head><meta charset=\"utf-8\"></head><body></body></html>",
            "text/html",
            "utf-8",
            null,
        )
        w.resumeTimers()
    }

    companion object {
        private const val TAG = "VolnaJs"
    }
}
