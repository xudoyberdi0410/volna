import java.util.Properties

plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

// android/local.properties (per machine, not in git).
val localProps = Properties().apply {
    val f = rootProject.file("local.properties")
    if (f.isFile) f.inputStream().use { load(it) }
}

/** A debug-only setting: env var first, then local.properties. */
fun devSetting(env: String, prop: String): String =
    System.getenv(env)?.takeIf { it.isNotEmpty() } ?: localProps.getProperty(prop, "")

fun quoted(s: String) = "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\""

android {
    namespace = "uz.khudoberdi.volna"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        buildConfig = true
    }

    defaultConfig {
        applicationId = "uz.khudoberdi.volna"
        // Media3 foreground media service, WebView JS engine.
        minSdk = maxOf(26, flutter.minSdkVersion)
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        versionCode = flutter.versionCode
        versionName = flutter.versionName
        buildConfigField("String", "DEV_PROXY", "\"\"")
        buildConfigField("String", "DEV_STREAM_URL", "\"\"")
    }

    buildTypes {
        debug {
            // docs/mobile-build.md, "Emulator": YouTube Music is blocked here,
            // so debug builds may route through the host's SOCKS proxy
            // (VOLNA_DEV_PROXY=socks5h://10.0.2.2:1081). The core ignores
            // both in release builds as well.
            buildConfigField("String", "DEV_PROXY", quoted(devSetting("VOLNA_DEV_PROXY", "volna.devProxy")))
            buildConfigField(
                "String",
                "DEV_STREAM_URL",
                quoted(devSetting("VOLNA_DEV_STREAM_URL", "volna.devStreamUrl")),
            )
        }
        release {
            // Signed with the debug key until there is a release key
            // (docs/mobile-build.md).
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

dependencies {
    implementation("androidx.media3:media3-exoplayer:1.8.0")
    implementation("androidx.media3:media3-session:1.8.0")
}

// libvolna_core.so: scripts/build-core-mobile.sh android (cargo-ndk) into a
// per-variant jniLibs dir under build/. Debug variants get a debug core
// (devProxy / devStreamUrl work), profile and release a release core. Only
// the ABIs Flutter builds for (`--target-platform`), else arm64-v8a and
// x86_64. `-Pvolna.skipCore=true` skips cargo and packs what is there.
val repoRoot: File = rootProject.projectDir.resolve("../..").canonicalFile
val coreAbis: List<String> = (project.findProperty("target-platform") as String?)
    ?.split(",")
    ?.mapNotNull {
        when (it.trim()) {
            "android-arm64" -> "arm64-v8a"
            "android-x64" -> "x86_64"
            else -> null
        }
    }
    ?.takeIf { it.isNotEmpty() }
    ?: listOf("arm64-v8a", "x86_64")
val skipCore = (project.findProperty("volna.skipCore") as String?) == "true"

for (variant in listOf("debug", "profile", "release")) {
    val cap = variant.replaceFirstChar { it.uppercase() }
    val out = layout.buildDirectory.dir("volna-core/$variant/jniLibs").get().asFile
    val task = tasks.register<Exec>("buildVolnaCore$cap") {
        description = "Builds libvolna_core.so ($variant) for ${coreAbis.joinToString()}"
        enabled = !skipCore
        workingDir = repoRoot
        environment("VOLNA_ANDROID_ABIS", coreAbis.joinToString(" "))
        environment("VOLNA_JNILIBS", out.absolutePath)
        // rustup's toolchain (with the Android targets) before a system Rust.
        val cargoHome = System.getenv("CARGO_HOME") ?: "${System.getProperty("user.home")}/.cargo"
        environment("PATH", "$cargoHome/bin:${System.getenv("PATH")}")
        val args = mutableListOf("bash", "scripts/build-core-mobile.sh", "android")
        if (variant != "debug") args += "--release"
        commandLine(args)
    }
    android.sourceSets.getByName(variant).jniLibs.srcDir(out)
    tasks.matching { it.name == "merge${cap}JniLibFolders" || it.name == "merge${cap}NativeLibs" }
        .configureEach { dependsOn(task) }
}
