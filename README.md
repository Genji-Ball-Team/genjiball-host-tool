# Genji Ball Host Tool

The desktop app for hosts of [Genji Ball Ranked](https://genjiball.us). It runs next to Overwatch, watches your Workshop log folder and uploads ranked matches to the ranked server, so hosts don't have to upload anything by hand.

How ranked works end to end:

1. A host runs the **v1.3.3R** version of [Genji Ball: Community Edition](https://github.com/Genji-Ball-Team/GenjiBall-CE). It writes what happens in each match to the Workshop log file ([format](https://github.com/Genji-Ball-Team/GenjiBall-CE/blob/v1.3.3R/docs/ranked-log.md)).
2. This tool watches the log folder and uploads the matches.
3. The [ranked server](https://github.com/Genji-Ball-Team/genjiball-ranked) parses the log, rates each round and shows the results on the website.

**Status:** early. The app checks and saves your host token and finds the log folder; uploads and the ranked code generator are next (see the [issues](https://github.com/Genji-Ball-Team/genjiball-host-tool/issues)). There's no release yet.

- **Discord:** [discord.gg/sv9VVjh5pT](https://discord.gg/sv9VVjh5pT), the Genji Ball Ranked server

## Build it

You need Node.js 22 or newer and [Rust](https://rustup.rs) stable. On Windows, Rust needs the MSVC build tools ([Tauri's prerequisites](https://tauri.app/start/prerequisites/)).

```sh
npm ci
npm run dev     # run the app with hot reload
npm run build   # build the installer
```

| Command | Use |
|---|---|
| `npm run dev` | Run the app with hot reload |
| `npm run check` | Typecheck, lint and test (TypeScript and Rust). What CI runs |
| `npm run build` | Build the Windows installer into `src-tauri/target/release/bundle/nsis/` |
| `npm run icons` | Regenerate the app icons from `app-icon.svg` |

CI builds the installer for every push and PR; download it from the run's "installer" artifact.

## Layout

| Path | What |
|---|---|
| `src/` | The window: TypeScript and CSS, built with Vite |
| `src-tauri/` | The Rust side: file watching, uploads, settings. Tauri config in `tauri.conf.json` |
| `src-tauri/src/config.rs` | Every tunable and its default |
| `src-tauri/src/settings.rs`, `credentials.rs` | The settings file, and the host token (Credential Manager, or a DPAPI-encrypted file) |
| `src-tauri/src/server.rs` | Requests to the ranked server |
| `src-tauri/capabilities/` | What the window is allowed to call |

## Contributing

Read [AGENTS.md](AGENTS.md) first (it's written for AI agents, but it's the short version of the rules for everyone). Open an issue or a PR; the templates say what to include.

## License

See [LICENSE.md](LICENSE.md). There is no open-source license yet.
