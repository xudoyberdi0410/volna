package uz.khudoberdi.volna

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.webkit.CookieManager
import android.webkit.WebView
import android.webkit.WebViewClient

/**
 * Google sign-in in a WebView, for the YouTube session the stream resolver
 * needs. When the login lands on music.youtube.com with a session, the
 * `.youtube.com` / `.google.com` cookies are exported as a Netscape
 * cookies.txt and returned in [EXTRA_COOKIES].
 *
 * Google may refuse embedded browsers; the WebView drops the `wv` marker
 * from its user agent, which is what it checks first.
 */
class GoogleSignInActivity : Activity() {
    private lateinit var web: WebView
    private var done = false

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        web = WebView(this)
        setContentView(web)
        web.settings.javaScriptEnabled = true
        web.settings.domStorageEnabled = true
        web.settings.userAgentString = web.settings.userAgentString
            .replace("; wv)", ")")
            .replace(Regex(" Version/[\\d.]+"), "")
        CookieManager.getInstance().setAcceptCookie(true)
        CookieManager.getInstance().setAcceptThirdPartyCookies(web, true)
        web.webViewClient = object : WebViewClient() {
            override fun onPageFinished(view: WebView, url: String?) {
                val host = url?.let { Uri.parse(it).host } ?: return
                if (host.endsWith("youtube.com") && hasSession()) finishWithCookies()
            }
        }
        web.loadUrl(LOGIN_URL)
    }

    private fun hasSession(): Boolean {
        val c = CookieManager.getInstance().getCookie("https://music.youtube.com/") ?: return false
        return c.contains("SAPISID=") || c.contains("__Secure-3PAPISID=")
    }

    private fun finishWithCookies() {
        if (done) return
        done = true
        CookieManager.getInstance().flush()
        setResult(RESULT_OK, Intent().putExtra(EXTRA_COOKIES, exportCookies()))
        finish()
    }

    override fun onDestroy() {
        web.destroy()
        super.onDestroy()
    }

    companion object {
        const val EXTRA_COOKIES = "cookies"
        private const val LOGIN_URL =
            "https://accounts.google.com/ServiceLogin?service=youtube&passive=true" +
                "&continue=https%3A%2F%2Fmusic.youtube.com%2F"

        /** The WebView's cookies for YouTube and Google as Netscape cookies.txt. */
        fun exportCookies(): String {
            val cm = CookieManager.getInstance()
            // CookieManager only gives name=value: the domain comes from the
            // URL, and they are written as secure, valid for a year.
            val sources = listOf(
                "https://music.youtube.com/" to ".youtube.com",
                "https://www.youtube.com/" to ".youtube.com",
                "https://accounts.google.com/" to ".google.com",
                "https://www.google.com/" to ".google.com",
            )
            val expiry = System.currentTimeMillis() / 1000 + 365L * 24 * 3600
            val seen = HashSet<String>()
            val out = StringBuilder("# Netscape HTTP Cookie File\n")
            for ((url, domain) in sources) {
                val header = cm.getCookie(url) ?: continue
                for (part in header.split(";")) {
                    val eq = part.indexOf('=')
                    if (eq <= 0) continue
                    val name = part.substring(0, eq).trim()
                    val value = part.substring(eq + 1).trim()
                    if (!seen.add("$domain\t$name")) continue
                    out.append("$domain\tTRUE\t/\tTRUE\t$expiry\t$name\t$value\n")
                }
            }
            return out.toString()
        }
    }
}
