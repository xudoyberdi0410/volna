import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
import XCTest
@testable import VolnaKit

final class CookieTests: XCTestCase {
    func cookie(_ domain: String, _ name: String, expires: Date? = nil, httpOnly: Bool = false) -> HTTPCookie {
        var p: [HTTPCookiePropertyKey: Any] = [.domain: domain, .path: "/", .name: name, .value: "v-\(name)", .secure: "TRUE"]
        if let expires { p[.expires] = expires }
        if httpOnly { p[HTTPCookiePropertyKey("HttpOnly")] = "TRUE" }
        return HTTPCookie(properties: p)!
    }

    func testSessionCookiesAsNetscapeText() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let text = NetscapeCookies.format([
            cookie(".youtube.com", "SAPISID", expires: Date(timeIntervalSince1970: 1_900_000_000)),
            cookie(".google.com", "SID", expires: Date(timeIntervalSince1970: 1_900_000_000), httpOnly: true),
            cookie("accounts.google.com", "LSID"),
            cookie(".example.com", "x"),
            cookie(".youtube.com", "OLD", expires: Date(timeIntervalSince1970: 1_000)),
            cookie(".notyoutube.com", "y"),
        ], now: now)
        let lines = text.split(separator: "\n").map(String.init)
        XCTAssertEqual(lines[0], "# Netscape HTTP Cookie File")
        XCTAssertEqual(lines.count, 4, text)
        XCTAssertTrue(lines.contains(".youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tv-SAPISID"), text)
        XCTAssertTrue(lines.contains("accounts.google.com\tFALSE\t/\tTRUE\t0\tLSID\tv-LSID"), text)
        // `#HttpOnly_` needs a platform that keeps the flag (Darwin does).
        XCTAssertTrue(lines.contains { $0.hasSuffix(".google.com\tTRUE\t/\tTRUE\t1900000000\tSID\tv-SID") }, text)
        XCTAssertTrue(NetscapeCookies.signedIn(text))
        XCTAssertFalse(NetscapeCookies.signedIn("# Netscape HTTP Cookie File\n"))
    }
}
