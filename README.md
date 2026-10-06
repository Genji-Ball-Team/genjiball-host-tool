# Genji Ball Host Tool

The desktop app for hosts of [Genji Ball Ranked](https://genjiball.us). It runs next to Overwatch, watches your Workshop log folder and uploads ranked matches to the ranked server, so hosts don't have to upload anything by hand.

How ranked works end to end:

1. A host runs the **v1.3.3R** version of [Genji Ball: Community Edition](https://github.com/Genji-Ball-Team/GenjiBall-CE). It writes what happens in each match to the Workshop log file ([format](https://github.com/Genji-Ball-Team/GenjiBall-CE/blob/v1.3.3R/docs/ranked-log.md)).
2. This tool watches the log folder and uploads the matches.
3. The [ranked server](https://github.com/Genji-Ball-Team/genjiball-ranked) parses the log, rates each round and shows the results on the website.

**Status:** early. The app checks and saves your host token, watches the log folder, uploads ranked matches from the tray as the region you host in (EU or NA), lists what it uploaded, shows the current match and each uploaded one the way the site will (and why one won't count), lets the host go AFK without their rating changing, and builds the ranked code (see the [issues](https://github.com/Genji-Ball-Team/genjiball-host-tool/issues) for what's next). Releases are built by CI (see [Install](#install)).

- **Discord:** [discord.gg/sv9VVjh5pT](https://discord.gg/sv9VVjh5pT), the Genji Ball Ranked server

## Install

Download the `.exe` installer from the [Releases page](https://github.com/Genji-Ball-Team/genjiball-host-tool/releases) and run it. Windows only.

Windows SmartScreen will warn that it doesn't know the app, because the installer isn't code-signed. Click "More info", then "Run anyway".

From v0.2.0 on the app updates itself: it looks for a new version when it starts and every few hours, and a banner offers "Install and restart". There's also a "Check for updates" button. v0.1.0 has no updater, so if you have it, install v0.2.0 by hand once.

Then follow the [hosting guide](docs/hosting.md).

## Make a release

1. Bump `version` in `package.json` and in `src-tauri/Cargo.toml` (the app and the installer take it from `package.json`; run a `cargo check` so `Cargo.lock` follows).
2. Merge that to `main`.
3. Tag it `vX.Y.Z` (the same number) and push the tag.

The [release workflow](.github/workflows/release.yml) runs the checks, builds the installer, signs it and attaches it to a GitHub release for the tag, together with `latest.json`, which the installed apps read to find the new version. It fails if the tag and `package.json` disagree, or if the signing key is missing.

**The updater signing key.** Updates are signed, and the app only installs one signed with our key (its public half is in `src-tauri/tauri.conf.json`). The private key and its password are the repo secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. A backup of the private key is kept by the team outside the repo: whoever holds it can sign releases. If it's lost, installed apps can't update any more: a new key means everyone installs the next version by hand once.

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
| `src-tauri/src/updates.rs` | The self-update: checks GitHub releases, installs a signed update |
| `src-tauri/src/watcher.rs`, `log_scan.rs` | Which log files to upload, and when |
| `src-tauri/src/uploader.rs`, `uploads.rs` | The upload loop, and the record of what was uploaded (`uploads.json`) |
| `src-tauri/src/afk.rs` | Host AFK: the rounds that start while it's on, per match (`afk.json`), sent as `X-Host-Afk` |
| `src-tauri/src/lobby.rs`, `live_lobby.rs` | The live lobby: lists the host's lobby on the site while a ranked match is played |
| `src-tauri/src/release.rs`, `ranked_code.rs` | The ranked code: the GenjiBall-CE release it's built on, and the rank tags filled in |
| `src-tauri/src/logging.rs`, `diagnostics.rs` | The tool's own log, and the diagnostics export for bug reports |
| `src-tauri/src/debug.rs`, `src/debug-view.ts` | The debug panel: the latest uploads and dry runs, and the newest log's last events |
| `src-tauri/capabilities/` | What the window is allowed to call |

## Contributing

Read [AGENTS.md](AGENTS.md) first (it's written for AI agents, but it's the short version of the rules for everyone). Open an issue or a PR; the templates say what to include.

## License

See [LICENSE.md](LICENSE.md). There is no open-source license yet.
