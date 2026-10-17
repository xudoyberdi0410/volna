#ifndef RUNNER_WEB_LOGIN_H_
#define RUNNER_WEB_LOGIN_H_

#include <flutter_linux/flutter_linux.h>

// "volna/login": a sign-in window with a web view (Yandex Music import).
// WebKitGTK is loaded with dlopen, so the app starts without it and the
// Dart side falls back to pasting the token by hand.
//
// Methods:
// - `available` → bool: WebKitGTK (4.1 or 4.0) could be loaded.
// - `open {url, title, match}` → the first URL the page reaches that contains
//   `match` (e.g. "access_token="), or null when the window is closed first.
//   One window at a time; a second `open` focuses it and answers null.
void web_login_register(FlBinaryMessenger* messenger, GtkWindow* parent);

#endif  // RUNNER_WEB_LOGIN_H_
