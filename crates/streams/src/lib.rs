//! Audio stream resolution via a yt-dlp sidecar.
//!
//! Resolves a `videoId` to a googlevideo audio URL (through the same proxy
//! as everything else, since the URLs are IP-bound), caches results until
//! their `expire`, and backs the `volna://stream/<videoId>` protocol with
//! HTTP Range support. Also handles yt-dlp self-updates.
