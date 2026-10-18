package uz.khudoberdi.volna.core

/**
 * The JNI entry points of `libvolna_core.so` (`crates/core/src/ffi/jni.rs`,
 * feature `jni`). The handle is a `VolnaCore *`; the core lives for the
 * whole process once started.
 */
internal object VolnaCoreNative {
    /** What the core calls back, from its own threads; must not block. */
    interface Host {
        fun onPlayerCommand(json: String)
        fun onJsRequest(id: Long, json: String)
        fun onCallResult(token: Long, ok: Boolean, json: String)
        fun onProtoResult(token: Long, status: Int, body: ByteArray)
    }

    init {
        System.loadLibrary("volna_core")
    }

    /** Starts the core once (`volna_core_init` config JSON); later calls return the same handle. */
    @JvmStatic external fun nativeStart(config: String, host: Host): Long
    @JvmStatic external fun nativeCall(handle: Long, token: Long, cmd: String, args: String)
    @JvmStatic external fun nativeProto(handle: Long, token: Long, uri: String)
    @JvmStatic external fun nativePlayerReport(handle: Long, json: String): Boolean
    @JvmStatic external fun nativeJsResult(handle: Long, id: Long, ok: Boolean, json: String): Boolean
    /** `volna_core_js_prelude()`: loaded once into the JS page. */
    @JvmStatic external fun nativeJsPrelude(): String
    /** Drops the Flutter engine's callbacks (reply, event, proto). */
    @JvmStatic external fun nativeDetachUi(handle: Long)
}
