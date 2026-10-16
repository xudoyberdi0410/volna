# Volna UI

Frontend of Volna, a fork of [Monochrome](https://github.com/monochrome-music/monochrome)
(Apache-2.0, see `license` and `NOTICE`). The pristine upstream copy lives in `../ui-upstream`.

```sh
bun install
bun run dev        # Vite dev server on :5173 (used by `tauri dev`)
bun run build      # production bundle in dist/ (used by `tauri build`)
bun run test:node  # pure-logic tests without a browser
bun run tauri dev  # run the desktop app (from this directory)
```

See `../docs/STRUCTURE.md` for the layout and the list of TIDAL-specific spots.
