//! Prints what the parsers make of a saved InnerTube response, as JSON:
//! `cargo run -p volna-ytm --example dump_fixture -- search_all crates/ytm/tests/fixtures/search_all.json`
//!
//! Handy when YouTube changes a layout: capture the new response, dump it,
//! compare with what the fixture tests expect.

use serde::Serialize;
use volna_ytm::{SearchFilter, parsers};

fn show<T: Serialize>(v: T) -> String {
    serde_json::to_string_pretty(&v).expect("serialize")
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(kind), Some(path)) = (args.next(), args.next()) else {
        eprintln!("usage: dump_fixture <kind> <file.json>");
        std::process::exit(2);
    };
    let raw = std::fs::read_to_string(&path).expect("read fixture");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("fixture is JSON");
    let out = match kind.as_str() {
        "search_all" => show(parsers::parse_search(&v, SearchFilter::All)),
        "search_songs" => show(parsers::parse_search(&v, SearchFilter::Songs)),
        "search_albums" => show(parsers::parse_search(&v, SearchFilter::Albums)),
        "search_artists" => show(parsers::parse_search(&v, SearchFilter::Artists)),
        "search_playlists" => show(parsers::parse_search(&v, SearchFilter::Playlists)),
        "search_videos" => show(parsers::parse_search(&v, SearchFilter::Videos)),
        "search_cont" => show(parsers::parse_search_continuation(&v, SearchFilter::Songs)),
        "suggestions" => show(parsers::parse_suggestions(&v)),
        "watch" => show(parsers::parse_watch(&v)),
        "watch_cont" => show(parsers::parse_watch_continuation(&v)),
        "album" => show(parsers::parse_album(&v, "MPRE")),
        "artist" => show(parsers::parse_artist(&v, "UC")),
        "playlist" => show(parsers::parse_playlist(&v, "PL")),
        "playlist_cont" => show(parsers::parse_playlist_continuation(&v)),
        "feed" => show(parsers::parse_feed(&v)),
        "moods" => show(parsers::parse_moods(&v)),
        "related" => show(parsers::parse_related(&v)),
        "lyrics" => show(parsers::parse_lyrics(&v)),
        other => {
            eprintln!("unknown kind {other}");
            std::process::exit(2);
        }
    };
    println!("{out}");
}
