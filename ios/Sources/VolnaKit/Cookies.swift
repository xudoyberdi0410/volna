import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// The signed-in Google session as the core's stream resolver takes it
/// (`youtubeCookies`: a Netscape cookies.txt, `CookieJar::from_netscape`).
public enum NetscapeCookies {
    /// The domains that make up the session.
    public static func isSession(_ domain: String) -> Bool {
        let d = domain.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "."))
        return ["youtube.com", "google.com"].contains { d == $0 || d.hasSuffix("." + $0) }
    }

    /// `cookies` of the session domains as cookies.txt (sorted, so the text
    /// is stable). Expired ones are left out.
    public static func format(_ cookies: [HTTPCookie], now: Date = Date()) -> String {
        let lines = cookies
            .filter { isSession($0.domain) }
            .filter { c in c.expiresDate.map { $0 > now } ?? true }
            .sorted { ($0.domain, $0.name, $0.path) < ($1.domain, $1.name, $1.path) }
            .map { c -> String in
                let sub = c.domain.hasPrefix(".") ? "TRUE" : "FALSE"
                let expires = c.expiresDate.map { String(Int($0.timeIntervalSince1970)) } ?? "0"
                let line = [c.domain, sub, c.path, c.isSecure ? "TRUE" : "FALSE", expires, c.name, c.value]
                    .joined(separator: "\t")
                return c.isHTTPOnly ? "#HttpOnly_" + line : line
            }
        return (["# Netscape HTTP Cookie File"] + lines).joined(separator: "\n") + "\n"
    }

    /// True when the text holds a signed-in YouTube session: `LOGIN_INFO`
    /// and one of the SAPISID cookies, as the core (and yt-dlp) checks.
    public static func signedIn(_ text: String) -> Bool {
        var names = Set<Substring>()
        for line in text.split(separator: "\n") {
            let l = line.hasPrefix("#HttpOnly_") ? line.dropFirst("#HttpOnly_".count) : line
            let f = l.split(separator: "\t", omittingEmptySubsequences: false)
            guard f.count >= 7, f[0].lowercased().hasSuffix("youtube.com") else { continue }
            names.insert(f[5])
        }
        return names.contains("LOGIN_INFO")
            && !names.isDisjoint(with: ["SAPISID", "__Secure-1PAPISID", "__Secure-3PAPISID"])
    }
}
