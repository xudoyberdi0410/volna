//! JNI for the Android app (feature `jni`). The Kotlin side
//! (`uz.khudoberdi.volna.core.VolnaCoreNative`) starts the one core of the
//! process, so playback belongs to the media service and not to the Flutter
//! isolate: the service drives the platform player and the WebView JS
//! engine through here and keeps working with no activity. Flutter attaches
//! to the same handle with `dart:ffi` (the handle travels as a `long` over a
//! method channel) and owns the reply / event / proto callbacks, which
//! `nativeDetachUi` clears before the Flutter engine goes away.
//!
//! The host object implements `onPlayerCommand(String)`,
//! `onJsRequest(long, String)`, `onCallResult(long, boolean, String)` and
//! `onProtoResult(long, int, byte[])`. They are called from core threads
//! (attached to the JVM for good) and must not block.

use std::ffi::{CString, c_char, c_void};
use std::sync::{Arc, Mutex, RwLock};

use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString, JValue};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jlong};
use jni::{JNIEnv, JavaVM};
use serde_json::Value;

use super::{
    VolnaCore, borrowed, handle, set, volna_core_free_string, volna_core_init,
    volna_core_js_prelude, volna_core_js_result, volna_core_last_error, volna_core_player_report,
    volna_core_set_js_callback, volna_core_set_player_callback,
};

/// The process's core once started (a `*mut VolnaCore`; never shut down).
static CORE: Mutex<usize> = Mutex::new(0);

struct Host {
    vm: JavaVM,
    obj: GlobalRef,
}

static HOST: RwLock<Option<Arc<Host>>> = RwLock::new(None);

fn host() -> Option<Arc<Host>> {
    HOST.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Runs `f` with this thread's JNI env inside a local frame, so references
/// made on long-lived core threads do not pile up.
fn with_host(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<()>) {
    let Some(host) = host() else {
        return;
    };
    let Ok(mut env) = host.vm.attach_current_thread_permanently() else {
        tracing::error!("cannot attach a core thread to the JVM");
        return;
    };
    let result = env.with_local_frame(8, |env| f(env, host.obj.as_obj()));
    if let Err(e) = result {
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        tracing::error!("JNI callback failed: {e}");
    }
}

/// Takes over a string the core handed to a callback.
fn take(s: *mut c_char) -> String {
    // SAFETY: NULL or a C string from `c_string`, freed once here.
    let out = unsafe { borrowed(s) }.unwrap_or_default().to_owned();
    unsafe { volna_core_free_string(s) };
    out
}

extern "C" fn on_player(_user: *mut c_void, cmd: *mut c_char) {
    let cmd = take(cmd);
    with_host(|env, obj| {
        let s = env.new_string(&cmd)?;
        env.call_method(
            obj,
            "onPlayerCommand",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&s)],
        )?;
        Ok(())
    });
}

extern "C" fn on_js(_user: *mut c_void, id: u64, request: *mut c_char) {
    let request = take(request);
    with_host(|env, obj| {
        let s = env.new_string(&request)?;
        env.call_method(
            obj,
            "onJsRequest",
            "(JLjava/lang/String;)V",
            &[JValue::Long(id as jlong), JValue::Object(&s)],
        )?;
        Ok(())
    });
}

fn string(env: &mut JNIEnv, s: &JString) -> Option<String> {
    if s.is_null() {
        return None;
    }
    env.get_string(s).ok().map(Into::into)
}

fn core_ref<'a>(h: jlong) -> Option<&'a VolnaCore> {
    // SAFETY: handles come from `nativeStart` and live for the process.
    unsafe { handle(h as usize as *mut VolnaCore) }
}

/// Starts the core once per process (later calls return the same handle;
/// `config` is the `volna_core_init` JSON) and makes `host` the platform
/// side. Throws `IllegalStateException` when the core cannot start.
#[unsafe(no_mangle)]
pub extern "system" fn Java_uz_khudoberdi_volna_core_VolnaCoreNative_nativeStart<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    config: JString<'l>,
    host_obj: JObject<'l>,
) -> jlong {
    let mut core = CORE.lock().unwrap_or_else(|e| e.into_inner());
    if *core == 0 {
        logcat::init();
        let Some(config) = string(&mut env, &config) else {
            let _ = env.throw_new("java/lang/IllegalArgumentException", "config is null");
            return 0;
        };
        let c = CString::new(config).unwrap_or_default();
        // SAFETY: a valid C string.
        let h = unsafe { volna_core_init(c.as_ptr()) };
        if h.is_null() {
            let why = take(volna_core_last_error());
            let _ = env.throw_new("java/lang/IllegalStateException", why);
            return 0;
        }
        *core = h as usize;
    }
    let h = *core as *mut VolnaCore;
    if !host_obj.is_null() {
        match (env.get_java_vm(), env.new_global_ref(&host_obj)) {
            (Ok(vm), Ok(obj)) => {
                *HOST.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(Host { vm, obj }));
                // SAFETY: a live handle; the callbacks are plain functions.
                unsafe {
                    volna_core_set_player_callback(h, Some(on_player), std::ptr::null_mut());
                    volna_core_set_js_callback(h, Some(on_js), std::ptr::null_mut());
                }
            }
            _ => tracing::error!("cannot keep the JNI host"),
        }
    }
    h as usize as jlong
}

/// Runs a command for the host; the answer goes to `onCallResult(token, …)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_uz_khudoberdi_volna_core_VolnaCoreNative_nativeCall<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
    token: jlong,
    cmd: JString<'l>,
    args: JString<'l>,
) {
    let Some(h) = core_ref(handle) else {
        return;
    };
    let cmd = string(&mut env, &cmd).unwrap_or_default();
    let args = string(&mut env, &args)
        .and_then(|a| serde_json::from_str(&a).ok())
        .unwrap_or(Value::Null);
    let core = h.core.clone();
    h.spawn(async move {
        let (ok, json) = match core.call(&cmd, &args).await {
            Ok(v) => (true, v.to_string()),
            Err(e) => (false, Value::String(e).to_string()),
        };
        with_host(|env, obj| {
            let s = env.new_string(&json)?;
            env.call_method(
                obj,
                "onCallResult",
                "(JZLjava/lang/String;)V",
                &[
                    JValue::Long(token),
                    JValue::Bool(ok.into()),
                    JValue::Object(&s),
                ],
            )?;
            Ok(())
        });
    });
}

/// A `volna://` GET for the host (cover art for the media session); the
/// answer goes to `onProtoResult(token, status, body)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_uz_khudoberdi_volna_core_VolnaCoreNative_nativeProto<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
    token: jlong,
    uri: JString<'l>,
) {
    let Some(h) = core_ref(handle) else {
        return;
    };
    let uri = string(&mut env, &uri).unwrap_or_default();
    let core = h.core.clone();
    h.spawn(async move {
        let resp = match http::Request::get(uri.as_str()).body(Vec::new()) {
            Ok(req) => core.proto(req).await,
            Err(e) => crate::protocol::json_error(http::StatusCode::BAD_REQUEST, &e.to_string()),
        };
        let status = i32::from(resp.status().as_u16());
        let body = resp.into_body();
        with_host(|env, obj| {
            let bytes: JByteArray = env.byte_array_from_slice(&body)?;
            env.call_method(
                obj,
                "onProtoResult",
                "(JI[B)V",
                &[
                    JValue::Long(token),
                    JValue::Int(status),
                    JValue::Object(&bytes),
                ],
            )?;
            Ok(())
        });
    });
}

/// `volna_core_player_report`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_uz_khudoberdi_volna_core_VolnaCoreNative_nativePlayerReport<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
    report: JString<'l>,
) -> jboolean {
    let Some(report) = string(&mut env, &report).and_then(|r| CString::new(r).ok()) else {
        return JNI_FALSE;
    };
    // SAFETY: a live handle and a valid string.
    let ok =
        unsafe { volna_core_player_report(handle as usize as *mut VolnaCore, report.as_ptr()) };
    if ok { JNI_TRUE } else { JNI_FALSE }
}

/// `volna_core_js_result`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_uz_khudoberdi_volna_core_VolnaCoreNative_nativeJsResult<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
    id: jlong,
    ok: jboolean,
    result: JString<'l>,
) -> jboolean {
    let result = string(&mut env, &result).unwrap_or_else(|| "null".into());
    let result = CString::new(result.replace('\0', "")).unwrap_or_default();
    // SAFETY: a live handle and a valid string.
    let done = unsafe {
        volna_core_js_result(
            handle as usize as *mut VolnaCore,
            id as u64,
            ok == JNI_TRUE,
            result.as_ptr(),
        )
    };
    if done { JNI_TRUE } else { JNI_FALSE }
}

/// `volna_core_js_prelude`: the script the WebView loads once.
#[unsafe(no_mangle)]
pub extern "system" fn Java_uz_khudoberdi_volna_core_VolnaCoreNative_nativeJsPrelude<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
) -> JString<'l> {
    // SAFETY: a static NUL-terminated string owned by the library.
    let prelude = unsafe { borrowed(volna_core_js_prelude()) }.unwrap_or_default();
    env.new_string(prelude).unwrap_or_default()
}

/// The Flutter engine is going away: drop its callbacks (reply, event,
/// proto) before its isolate does. Playback is not affected.
#[unsafe(no_mangle)]
pub extern "system" fn Java_uz_khudoberdi_volna_core_VolnaCoreNative_nativeDetachUi<'l>(
    _env: JNIEnv<'l>,
    _class: JClass<'l>,
    handle: jlong,
) {
    if let Some(h) = core_ref(handle) {
        set(&h.reply, None, std::ptr::null_mut());
        set(&h.event, None, std::ptr::null_mut());
        set(&h.proto, None, std::ptr::null_mut());
    }
}

/// `tracing` into logcat (tag `volna-core`): stderr goes nowhere on Android.
mod logcat {
    #[cfg(target_os = "android")]
    pub fn init() {
        use std::ffi::{CString, c_char, c_int};
        use std::io::Write;

        #[link(name = "log")]
        unsafe extern "C" {
            fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
        }

        /// One formatted event; written to logcat when dropped.
        struct Line(Vec<u8>);
        impl Write for Line {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl Drop for Line {
            fn drop(&mut self) {
                let text = String::from_utf8_lossy(&self.0);
                let text = text.trim_end();
                if text.is_empty() {
                    return;
                }
                if let Ok(t) = CString::new(text.replace('\0', "")) {
                    // SAFETY: valid C strings. Priority INFO; the level is in the text.
                    unsafe { __android_log_write(4, c"volna-core".as_ptr(), t.as_ptr()) };
                }
            }
        }
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
            .with_ansi(false)
            .without_time()
            .with_writer(|| Line(Vec::new()))
            .try_init();
    }

    #[cfg(not(target_os = "android"))]
    pub fn init() {}
}
