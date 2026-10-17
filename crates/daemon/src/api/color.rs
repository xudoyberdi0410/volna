//! `color_palette{imageUri}`: dominant and accent colours of a cover, for the
//! player background. Computed here so the UI stays thin and portable.
//!
//! The image is fetched through [`ImageCache`] (the shared client, so the
//! proxy applies, and the same disk cache as `volna://img/`), shrunk to at
//! most [`SAMPLE_PX`] px and split into [`SWATCHES`] boxes by median cut.
//! Results are kept per source URL in a small in-memory LRU.
//!
//! `imageUri` is either a `volna://img/?u=<url>[&s=<px>]` URI (also the
//! Windows form `http://volna.localhost/img/?u=…`) or the plain cover URL.
//!
//! Result (camelCase):
//! ```json
//! { "dominant": "#1a2b3c", "accent": "#e0a040",
//!   "onDominant": "#ffffff", "onAccent": "#000000", "dark": true,
//!   "swatches": [ { "color": "#1a2b3c", "share": 0.62 }, … ] }
//! ```
//! `dark` is whether `dominant` reads as dark (white text on it);
//! `swatches` are sorted by share (0..1, sums to 1).

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use volna_streams::{ImageCache, Network, StreamError};

use crate::dispatch::{arg, out};

/// Longest side of the image the palette is computed from.
const SAMPLE_PX: u32 = 64;
/// Cover size requested when the URI does not name one.
const FETCH_PX: u32 = 120;
/// Median-cut boxes.
const SWATCHES: usize = 8;
/// RGB distance under which two boxes are one swatch.
const MERGE_DISTANCE: f32 = 24.0;
/// Palettes kept in memory.
const CACHE_ENTRIES: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub dominant: String,
    pub accent: String,
    pub on_dominant: String,
    pub on_accent: String,
    pub dark: bool,
    pub swatches: Vec<Swatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Swatch {
    pub color: String,
    pub share: f32,
}

/// Runs `color_palette`; `None` for any other command.
pub async fn call<N: Network>(
    palettes: &Palettes,
    images: &ImageCache<N>,
    cmd: &str,
    a: &Value,
) -> Option<Result<Value, String>> {
    if cmd != "color_palette" {
        return None;
    }
    Some(match arg::<String>(a, "imageUri") {
        Ok(uri) => out(palettes.get(images, &uri).await),
        Err(e) => Err(format!("bad_args: {e}")),
    })
}

/// LRU of computed palettes, keyed by the cover URL (any size of the same
/// cover shares the entry).
pub struct Palettes {
    inner: Mutex<Lru>,
}

struct Lru {
    map: HashMap<String, (Palette, u64)>,
    tick: u64,
    cap: usize,
}

impl Default for Palettes {
    fn default() -> Self {
        Self::with_capacity(CACHE_ENTRIES)
    }
}

impl Palettes {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Mutex::new(Lru {
                map: HashMap::new(),
                tick: 0,
                cap: cap.max(1),
            }),
        }
    }

    fn cached(&self, key: &str) -> Option<Palette> {
        let mut lru = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        lru.tick += 1;
        let tick = lru.tick;
        let (p, used) = lru.map.get_mut(key)?;
        *used = tick;
        Some(p.clone())
    }

    fn insert(&self, key: String, p: Palette) {
        let mut lru = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        lru.tick += 1;
        let tick = lru.tick;
        if lru.map.len() >= lru.cap
            && !lru.map.contains_key(&key)
            && let Some(oldest) = lru
                .map
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
        {
            lru.map.remove(&oldest);
        }
        lru.map.insert(key, (p, tick));
    }

    /// Palette of the cover behind `image_uri`.
    pub async fn get<N: Network>(
        &self,
        images: &ImageCache<N>,
        image_uri: &str,
    ) -> Result<Palette, String> {
        let (url, size) = source(image_uri)?;
        if let Some(p) = self.cached(&url) {
            return Ok(p);
        }
        let img = images.get(&url, Some(size)).await.map_err(|e| match e {
            StreamError::InvalidUrl(_) | StreamError::HostNotAllowed(_) => {
                format!("bad_args: {e}")
            }
            other => format!("upstream: {other}"),
        })?;
        let p = tokio::task::spawn_blocking(move || palette_from_bytes(&img.bytes))
            .await
            .map_err(|e| format!("internal: {e}"))??;
        self.insert(url, p.clone());
        Ok(p)
    }
}

/// Cover URL and fetch size from a `volna://img/` URI or a plain URL.
fn source(uri: &str) -> Result<(String, u32), String> {
    let parsed = url::Url::parse(uri).map_err(|e| format!("bad_args: imageUri {uri:?}: {e}"))?;
    let volna_img = match (parsed.scheme(), parsed.host_str()) {
        ("volna", Some(h)) => h.eq_ignore_ascii_case("img"),
        ("http" | "https", Some(h)) if h.eq_ignore_ascii_case("volna.localhost") => {
            parsed.path().trim_matches('/') == "img"
        }
        _ => false,
    };
    if !volna_img {
        return Ok((uri.to_owned(), FETCH_PX));
    }
    let q = |name: &str| {
        parsed
            .query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    };
    let url = q("u").ok_or("bad_args: imageUri without u=")?;
    let size = q("s").and_then(|s| s.parse().ok()).unwrap_or(FETCH_PX);
    Ok((url, size))
}

/// Decodes an image (JPEG, PNG, WebP) and computes its palette.
pub fn palette_from_bytes(bytes: &[u8]) -> Result<Palette, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("upstream: bad image: {e}"))?;
    let small = img.thumbnail(SAMPLE_PX, SAMPLE_PX).to_rgba8();
    let pixels: Vec<[u8; 3]> = small
        .pixels()
        .filter(|p| p.0[3] >= 128)
        .map(|p| [p.0[0], p.0[1], p.0[2]])
        .collect();
    palette_from_pixels(&pixels).ok_or_else(|| "upstream: image has no opaque pixels".into())
}

/// Median cut over `pixels`; `None` when empty.
pub fn palette_from_pixels(pixels: &[[u8; 3]]) -> Option<Palette> {
    if pixels.is_empty() {
        return None;
    }
    let total = pixels.len() as f32;
    let mut boxes = median_cut(pixels.to_vec(), SWATCHES);
    boxes.sort_by_key(|b| std::cmp::Reverse(b.len()));
    // Boxes of nearly the same colour count as one swatch.
    let mut swatches: Vec<(Rgb, f32)> = Vec::new();
    for b in &boxes {
        let (c, share) = (Rgb::mean(b), b.len() as f32 / total);
        match swatches
            .iter_mut()
            .find(|(s, _)| s.distance(c) < MERGE_DISTANCE)
        {
            Some((s, w)) => {
                *s = s.mix(c.0, share / (*w + share));
                *w += share;
            }
            None => swatches.push((c, share)),
        }
    }
    swatches.sort_by(|a, b| b.1.total_cmp(&a.1));

    let dominant = swatches[0].0;
    let accent = swatches
        .iter()
        .skip(1)
        .filter(|(c, share)| *share >= 0.03 && c.distance(dominant) >= 60.0)
        .max_by(|a, b| accent_score(a).total_cmp(&accent_score(b)))
        .map(|(c, _)| *c)
        .unwrap_or_else(|| {
            if dominant.luminance() < 0.18 {
                dominant.mix([255, 255, 255], 0.45)
            } else {
                dominant.mix([0, 0, 0], 0.45)
            }
        });

    Some(Palette {
        dominant: dominant.hex(),
        accent: accent.hex(),
        on_dominant: dominant.on().hex(),
        on_accent: accent.on().hex(),
        dark: dominant.on() == Rgb::WHITE,
        swatches: swatches
            .iter()
            .map(|(c, share)| Swatch {
                color: c.hex(),
                share: (share * 1000.0).round() / 1000.0,
            })
            .collect(),
    })
}

/// Vivid and not tiny wins.
fn accent_score((c, share): &(Rgb, f32)) -> f32 {
    c.chroma() * share.sqrt()
}

/// Splits the box with the widest channel range at its median until there
/// are `n` boxes or nothing is left to split.
fn median_cut(pixels: Vec<[u8; 3]>, n: usize) -> Vec<Vec<[u8; 3]>> {
    let mut boxes = vec![pixels];
    while boxes.len() < n {
        let Some((i, ch, range)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(i, b)| {
                let (ch, range) = widest_channel(b);
                (i, ch, range)
            })
            .max_by_key(|&(i, _, range)| (range as usize) * boxes[i].len())
        else {
            break;
        };
        if range == 0 {
            break;
        }
        let mut b = boxes.swap_remove(i);
        b.sort_unstable_by_key(|p| p[ch]);
        // Cut at the median value, keeping equal values on one side.
        let median = b[b.len() / 2][ch];
        let mut cut = b.partition_point(|p| p[ch] <= median);
        if cut == b.len() {
            cut = b.partition_point(|p| p[ch] < median);
        }
        let upper = b.split_off(cut);
        boxes.push(b);
        boxes.push(upper);
    }
    boxes
}

fn widest_channel(b: &[[u8; 3]]) -> (usize, u8) {
    (0..3)
        .map(|ch| {
            let (lo, hi) = b
                .iter()
                .fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[ch]), hi.max(p[ch])));
            (ch, hi.saturating_sub(lo))
        })
        .max_by_key(|&(_, r)| r)
        .unwrap_or((0, 0))
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rgb([u8; 3]);

impl Rgb {
    const WHITE: Rgb = Rgb([255, 255, 255]);
    const BLACK: Rgb = Rgb([0, 0, 0]);

    fn mean(px: &[[u8; 3]]) -> Self {
        let mut sum = [0u64; 3];
        for p in px {
            for ch in 0..3 {
                sum[ch] += u64::from(p[ch]);
            }
        }
        let n = px.len().max(1) as u64;
        Rgb(sum.map(|s| ((s + n / 2) / n) as u8))
    }

    fn hex(self) -> String {
        let [r, g, b] = self.0;
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    /// Max minus min channel, 0..1.
    fn chroma(self) -> f32 {
        let [r, g, b] = self.0;
        f32::from(r.max(g).max(b) - r.min(g).min(b)) / 255.0
    }

    fn distance(self, o: Rgb) -> f32 {
        (0..3)
            .map(|i| (f32::from(self.0[i]) - f32::from(o.0[i])).powi(2))
            .sum::<f32>()
            .sqrt()
    }

    fn mix(self, to: [u8; 3], t: f32) -> Rgb {
        Rgb(std::array::from_fn(|i| {
            (f32::from(self.0[i]) * (1.0 - t) + f32::from(to[i]) * t).round() as u8
        }))
    }

    /// WCAG relative luminance.
    fn luminance(self) -> f32 {
        let lin = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let [r, g, b] = self.0;
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    /// Black or white, whichever contrasts more.
    fn on(self) -> Rgb {
        let l = self.luminance();
        let vs_white = 1.05 / (l + 0.05);
        let vs_black = (l + 0.05) / 0.05;
        if vs_white >= vs_black {
            Rgb::WHITE
        } else {
            Rgb::BLACK
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `w×h` image filled by `f(x, y)`, encoded as PNG.
    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb(f(x, y)));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn two_colours_split_into_dominant_and_accent() {
        // 70 % dark navy, 30 % orange.
        let bytes = png(
            100,
            100,
            |x, _| {
                if x < 70 { [20, 30, 70] } else { [240, 140, 20] }
            },
        );
        let p = palette_from_bytes(&bytes).unwrap();
        assert_eq!(p.dominant, "#141e46");
        assert_eq!(p.accent, "#f08c14");
        assert!(p.dark);
        assert_eq!(p.on_dominant, "#ffffff");
        assert_eq!(p.on_accent, "#000000");
        let share: f32 = p.swatches.iter().map(|s| s.share).sum();
        assert!((share - 1.0).abs() < 0.01, "{share}");
        assert!((p.swatches[0].share - 0.7).abs() < 0.03, "{:?}", p.swatches);
    }

    #[test]
    fn vivid_small_patch_beats_dull_large_one_for_accent() {
        // 60 % light grey background, 30 % mid grey, 10 % vivid green.
        let bytes = png(100, 100, |x, _| match x {
            0..60 => [220, 220, 220],
            60..90 => [120, 120, 120],
            _ => [30, 200, 60],
        });
        let p = palette_from_bytes(&bytes).unwrap();
        assert_eq!(p.dominant, "#dcdcdc");
        assert_eq!(p.accent, "#1ec83c");
        assert!(!p.dark);
        assert_eq!(p.on_dominant, "#000000");
    }

    #[test]
    fn flat_image_derives_an_accent() {
        let p = palette_from_pixels(&[[10, 10, 10]; 50]).unwrap();
        assert_eq!(p.dominant, "#0a0a0a");
        assert_ne!(p.accent, p.dominant);
        assert_eq!(p.swatches.len(), 1);
        let light = palette_from_pixels(&[[250, 250, 250]; 50]).unwrap();
        assert!(Rgb::WHITE.distance(hex_rgb(&light.accent)) > 60.0);
    }

    #[test]
    fn noise_gives_at_most_eight_swatches() {
        let bytes = png(64, 64, |x, y| {
            [(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8]
        });
        let p = palette_from_bytes(&bytes).unwrap();
        assert!(
            (2..=SWATCHES).contains(&p.swatches.len()),
            "{:?}",
            p.swatches
        );
        assert!(p.swatches.windows(2).all(|w| w[0].share >= w[1].share));
    }

    #[test]
    fn transparent_and_broken_images_fail() {
        assert!(palette_from_pixels(&[]).is_none());
        assert!(
            palette_from_bytes(b"not an image")
                .unwrap_err()
                .starts_with("upstream: ")
        );
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 0]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        assert!(palette_from_bytes(&buf.into_inner()).is_err());
    }

    fn hex_rgb(s: &str) -> Rgb {
        let v = u32::from_str_radix(&s[1..], 16).unwrap();
        Rgb([(v >> 16) as u8, (v >> 8) as u8, v as u8])
    }

    #[test]
    fn contrast_colour() {
        assert_eq!(Rgb::BLACK.on(), Rgb::WHITE);
        assert_eq!(Rgb::WHITE.on(), Rgb::BLACK);
        assert_eq!(Rgb([255, 255, 0]).on(), Rgb::BLACK);
        assert_eq!(Rgb([0, 0, 200]).on(), Rgb::WHITE);
    }

    #[test]
    fn uri_forms() {
        let lh3 = "https://lh3.googleusercontent.com/abc=w60-h60";
        assert_eq!(source(lh3).unwrap(), (lh3.to_owned(), FETCH_PX));
        let enc = "https%3A%2F%2Flh3.googleusercontent.com%2Fabc%3Dw60-h60";
        assert_eq!(
            source(&format!("volna://img/?u={enc}&s=300")).unwrap(),
            (lh3.to_owned(), 300)
        );
        assert_eq!(
            source(&format!("http://volna.localhost/img/?u={enc}")).unwrap(),
            (lh3.to_owned(), FETCH_PX)
        );
        assert!(
            source("volna://img/?s=3")
                .unwrap_err()
                .starts_with("bad_args")
        );
        assert!(source("not a uri").unwrap_err().starts_with("bad_args"));
    }

    fn pal(c: &str) -> Palette {
        palette_from_pixels(&[hex_rgb(c).0]).unwrap()
    }

    #[test]
    fn lru_evicts_least_recently_used() {
        let lru = Palettes::with_capacity(2);
        lru.insert("a".into(), pal("#000001"));
        lru.insert("b".into(), pal("#000002"));
        assert!(lru.cached("a").is_some()); // `b` is now the oldest
        lru.insert("c".into(), pal("#000003"));
        assert!(lru.cached("b").is_none());
        assert_eq!(lru.cached("a").unwrap().dominant, "#000001");
        assert_eq!(lru.cached("c").unwrap().dominant, "#000003");
    }

    #[tokio::test]
    async fn command_validates_without_network() {
        let state = crate::protocol::tests::test_state();
        let p = Palettes::default();
        assert!(
            call(&p, &state.images, "catalog_album", &json!({}))
                .await
                .is_none()
        );
        let e = call(&p, &state.images, "color_palette", &json!({})).await;
        assert!(e.unwrap().unwrap_err().starts_with("bad_args"));
        let e = call(
            &p,
            &state.images,
            "color_palette",
            &json!({"imageUri": "https://example.com/a.jpg"}),
        )
        .await;
        assert!(e.unwrap().unwrap_err().starts_with("bad_args"));
    }

    /// A real cover through the proxy, like `api_live`.
    #[tokio::test]
    #[ignore = "network: i.ytimg.com"]
    async fn api_live_color_palette() {
        let state = crate::protocol::tests::test_state();
        if let Ok(proxy) = std::env::var("VOLNA_TEST_PROXY") {
            crate::network::switch(&state, volna_net::ProxyConfig::Url(proxy))
                .await
                .expect("VOLNA_TEST_PROXY");
        }
        let p = Palettes::default();
        let uri = "volna://img/?u=https%3A%2F%2Fi.ytimg.com%2Fvi%2FdQw4w9WgXcQ%2Fhqdefault.jpg";
        let got = p.get(&state.images, uri).await.unwrap();
        println!("{}", serde_json::to_string(&got).unwrap());
        assert!(got.dominant.starts_with('#') && got.dominant.len() == 7);
        assert!(!got.swatches.is_empty());
    }
}
