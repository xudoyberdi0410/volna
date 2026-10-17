#include "web_login.h"

#include <dlfcn.h>

#include <cstring>

namespace {

// The few WebKitGTK functions we need, resolved at run time.
using WebViewNewFn = GtkWidget* (*)();
using LoadUriFn = void (*)(void* web_view, const gchar* uri);
using GetUriFn = const gchar* (*)(void* web_view);

struct WebKit {
  bool tried = false;
  void* handle = nullptr;
  WebViewNewFn web_view_new = nullptr;
  LoadUriFn load_uri = nullptr;
  GetUriFn get_uri = nullptr;
};

WebKit webkit;

bool load_webkit() {
  if (webkit.tried) return webkit.handle != nullptr;
  webkit.tried = true;
  // GTK3 builds only: Flutter's Linux embedder is GTK3.
  const char* names[] = {"libwebkit2gtk-4.1.so.0", "libwebkit2gtk-4.0.so.37"};
  for (const char* name : names) {
    void* h = dlopen(name, RTLD_NOW | RTLD_LOCAL);
    if (h == nullptr) continue;
    auto n = reinterpret_cast<WebViewNewFn>(dlsym(h, "webkit_web_view_new"));
    auto l = reinterpret_cast<LoadUriFn>(dlsym(h, "webkit_web_view_load_uri"));
    auto g = reinterpret_cast<GetUriFn>(dlsym(h, "webkit_web_view_get_uri"));
    if (n != nullptr && l != nullptr && g != nullptr) {
      webkit.handle = h;
      webkit.web_view_new = n;
      webkit.load_uri = l;
      webkit.get_uri = g;
      return true;
    }
    dlclose(h);
  }
  return false;
}

struct Login {
  GtkWidget* window = nullptr;
  FlMethodCall* call = nullptr;  // answered once
  gchar* match = nullptr;
};

Login* current = nullptr;
GtkWindow* parent_window = nullptr;

void finish(Login* login, const gchar* url) {
  if (login->call == nullptr) return;
  g_autoptr(FlValue) result =
      url == nullptr ? fl_value_new_null() : fl_value_new_string(url);
  fl_method_call_respond_success(login->call, result, nullptr);
  g_clear_object(&login->call);
}

void on_uri_changed(GObject* web_view, GParamSpec*, gpointer user_data) {
  auto* login = static_cast<Login*>(user_data);
  const gchar* uri = webkit.get_uri(web_view);
  if (uri == nullptr || login->match == nullptr ||
      strstr(uri, login->match) == nullptr) {
    return;
  }
  finish(login, uri);
  // Not from inside the web view's own signal.
  g_idle_add(
      [](gpointer w) -> gboolean {
        gtk_widget_destroy(GTK_WIDGET(w));
        g_object_unref(w);
        return G_SOURCE_REMOVE;
      },
      g_object_ref(login->window));
}

void on_destroy(GtkWidget*, gpointer user_data) {
  auto* login = static_cast<Login*>(user_data);
  finish(login, nullptr);
  g_free(login->match);
  if (current == login) current = nullptr;
  delete login;
}

FlValue* arg(FlValue* args, const char* key) {
  if (args == nullptr || fl_value_get_type(args) != FL_VALUE_TYPE_MAP) {
    return nullptr;
  }
  FlValue* v = fl_value_lookup_string(args, key);
  return v != nullptr && fl_value_get_type(v) == FL_VALUE_TYPE_STRING ? v
                                                                       : nullptr;
}

void open_login(FlMethodCall* call) {
  if (!load_webkit()) {
    fl_method_call_respond_error(call, "unavailable", "WebKitGTK not found",
                                 nullptr, nullptr);
    return;
  }
  FlValue* args = fl_method_call_get_args(call);
  FlValue* url = arg(args, "url");
  if (url == nullptr) {
    fl_method_call_respond_error(call, "bad_args", "url is required", nullptr,
                                 nullptr);
    return;
  }
  if (current != nullptr) {
    gtk_window_present(GTK_WINDOW(current->window));
    fl_method_call_respond_success(call, nullptr, nullptr);
    return;
  }
  FlValue* title = arg(args, "title");
  FlValue* match = arg(args, "match");

  auto* login = new Login();
  login->call = FL_METHOD_CALL(g_object_ref(call));
  login->match = g_strdup(match != nullptr ? fl_value_get_string(match)
                                           : "access_token=");
  current = login;

  GtkWidget* window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
  login->window = window;
  gtk_window_set_title(GTK_WINDOW(window),
                       title != nullptr ? fl_value_get_string(title) : "Вход");
  gtk_window_set_default_size(GTK_WINDOW(window), 480, 720);
  if (parent_window != nullptr) {
    gtk_window_set_transient_for(GTK_WINDOW(window), parent_window);
  }
  GtkWidget* view = webkit.web_view_new();
  gtk_container_add(GTK_CONTAINER(window), view);
  g_signal_connect(view, "notify::uri", G_CALLBACK(on_uri_changed), login);
  g_signal_connect(window, "destroy", G_CALLBACK(on_destroy), login);
  webkit.load_uri(view, fl_value_get_string(url));
  gtk_widget_show_all(window);
}

void method_cb(FlMethodChannel*, FlMethodCall* call, gpointer) {
  const gchar* name = fl_method_call_get_name(call);
  if (g_strcmp0(name, "available") == 0) {
    g_autoptr(FlValue) result = fl_value_new_bool(load_webkit());
    fl_method_call_respond_success(call, result, nullptr);
  } else if (g_strcmp0(name, "open") == 0) {
    open_login(call);
  } else {
    fl_method_call_respond_not_implemented(call, nullptr);
  }
}

}  // namespace

void web_login_register(FlBinaryMessenger* messenger, GtkWindow* parent) {
  parent_window = parent;
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  FlMethodChannel* channel =
      fl_method_channel_new(messenger, "volna/login", FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(channel, method_cb, nullptr,
                                            nullptr);
  // Lives as long as the window.
  g_object_set_data_full(G_OBJECT(parent), "volna-login-channel", channel,
                         g_object_unref);
}
