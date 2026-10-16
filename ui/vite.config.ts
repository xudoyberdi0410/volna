import { defineConfig } from 'vite';
import blobAssetPlugin from './vite-plugin-blob.js';
import svgUse from './vite-plugin-svg-use.js';
import { playwright } from '@vitest/browser-playwright';
import { execSync } from 'child_process';

function getGitCommitHash() {
    try {
        return execSync('git rev-parse --short HEAD').toString().trim();
    } catch {
        return 'unknown';
    }
}

// Set by `tauri dev` when running on a device/emulator that needs a LAN host.
const tauriDevHost = process.env.TAURI_DEV_HOST;

export default defineConfig(() => {
    const commitHash = getGitCommitHash();

    return {
        test: {
            // https://vitest.dev/guide/browser/
            browser: {
                enabled: true,
                provider: playwright({
                    launchOptions: {
                        args: ['--disable-web-security'],
                    },
                }),
                headless: !!process.env.HEADLESS,
                instances: [{ browser: 'chromium' }],
            },
        },
        base: './',
        // Keep Rust errors visible in the `tauri dev` terminal.
        clearScreen: false,
        envPrefix: ['VITE_', 'TAURI_ENV_'],
        define: {
            __COMMIT_HASH__: JSON.stringify(commitHash),
            __VITEST__: !!process.env.VITEST,
            'process.env.CI': JSON.stringify(process.env.CI ?? ''),
        },
        worker: {
            format: 'es',
        },
        resolve: {
            alias: {
                '!lucide': '/node_modules/lucide-static/icons',
                '!simpleicons': '/node_modules/simple-icons/icons',
                '!': '/node_modules',

                events: '/node_modules/events/events.js',
            },
        },
        optimizeDeps: {
            include: ['@capacitor/haptics', 'fuse.js'],
        },
        server: {
            // Tauri expects a fixed port (see src-tauri/tauri.conf.json devUrl).
            port: 5173,
            strictPort: true,
            host: tauriDevHost || false,
            hmr: tauriDevHost ? { protocol: 'ws', host: tauriDevHost, port: 5174 } : undefined,
            fs: {
                allow: ['.', 'node_modules'],
            },
            watch: {
                ignored: ['**/src-tauri/**'],
            },
        },
        build: {
            outDir: 'dist',
            emptyOutDir: true,
            sourcemap: false,
            // WebKitGTK (Linux) and WebView2 (Windows) are both modern engines.
            target: ['es2022', 'safari15', 'chrome105'],
            minify: 'esbuild',
            reportCompressedSize: false,
            rollupOptions: {
                treeshake: true,
            },
        },
        plugins: [blobAssetPlugin(), svgUse()],
    };
});
