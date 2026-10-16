//! YouTube Music InnerTube client (`WEB_REMIX`).
//!
//! Own implementation of the endpoints Volna needs: `search`, `browse`
//! (artist / album / playlist pages), `next` (radio queues such as
//! `RDAMVM<videoId>`), related tracks, lyrics and moods & genres.
//! All HTTP goes through the shared client from `volna-net` so the
//! configured proxy applies.
