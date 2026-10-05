# Genji Ball Host Tool

The desktop app ranked hosts run next to Overwatch: it watches the Workshop log folder, uploads ranked matches to the ranked server and builds the ranked Workshop code. Tauri 2: the work happens in Rust (`src-tauri/`), the window is a small TypeScript frontend (`src/`) built with Vite. Windows first, since Overwatch runs on PC.

## Commands

| Command | Use |
|---|---|
| `npm ci` | Install |
| `npm run dev` | Run the app with hot reload |
| `npm run check` | Typecheck, lint, `cargo fmt --check`, clippy (warnings are errors) and `cargo test`. What CI runs. **It must pass before a change is done.** |
| `npm run build` | Build the installer (`src-tauri/target/release/bundle/nsis/`) |
| `npm run icons` | Regenerate `src-tauri/icons/` from `app-icon.svg` |

You need Node.js 22+ and Rust stable (MSVC toolchain on Windows). CI runs `npm run check` and builds the installer on Windows for every push and PR (`.github/workflows/build.yml`).

## Where code goes

- **Rust** does the work that must keep running with the window closed or in the tray: watching files, parsing, the upload queue, talking to the server and GitHub, the credential store. Keep logic in small pure modules with unit tests, apart from the Tauri glue.
- **The frontend** only shows state and sends what the host typed. It calls Rust through `#[tauri::command]`s; mirror each command's return type as a TypeScript interface next to the call. No framework: plain TypeScript and DOM until there's a reason.
- Give the window only the permissions it uses (`src-tauri/capabilities/`), and keep the CSP in `tauri.conf.json` tight. Add a Tauri plugin only when a feature needs it.
- The app's look takes only its colours from genjiball.us (the same palette as the ranked site's `public/style.css`), not its layout.

## Config: no magic numbers

Every tunable lives in `src-tauri/src/config.rs` with its default and a comment saying what it does: server URL, timeouts, retry counts and backoff, poll intervals, how long a log must be quiet, cache times, feature flags. Code reads the config, never a literal.

- Each tunable becomes a setting the host can change (#11). Normal hosts see the server URL, token and region; everything else sits under "Advanced".
- A number a host can change is a `config::Tunable` (key, label, help, default, range) listed in `config::TUNABLES`: the window builds its Advanced field from it, and `settings.rs` checks the range when it's saved and when the file is read. Code reads it with `settings.get(&config::QUIET_SECS)` (or `.secs()`), never the default directly, and takes it from the settings each time it's used, so a change counts from the next poll or request. Tests may use `.default`.
- Unfinished features ship behind a flag, off by default.
- Debug settings come with the features (#11): a log level, a dry run that shows what would be uploaded, a debug panel and a diagnostics export.

## Settings and the host token

- Settings live in `settings.json` in the app's config folder (`%APPDATA%\us.genjiball.hosttool`), through `src-tauri/src/settings.rs`. A setting left at its default is stored as `null` (or, under `advanced`, left out), so a new default reaches every host. A file that can't be read, or holds a server URL or an Advanced value the window wouldn't take, pauses uploads (the defaults would be the live server and the real log folder) until it's fixed or a setting is changed.
- The host token, one per server URL, goes through `src-tauri/src/credentials.rs`: Windows Credential Manager first. When that refuses (a Credential Manager filled by the Xbox app's tokens answers "not enough memory", common on gaming PCs), the token goes in `tokens.json`, encrypted with DPAPI for the Windows user (`dpapi.rs`). A fallback entry takes precedence when reading, even if deleting an older native token fails; a successful native save removes the fallback entry.
- The token never goes in a plain file, a log, an error message, the window's state or a diagnostics export.
- The server checks a token with `GET /api/host/me` (genjiball-ranked `docs/api.md`). Only a `401` or `403` means a bad token; anything else is "couldn't check", and the token is saved anyway.
- What was uploaded is in `uploads.json` (`uploads.rs`): per server URL and file, the size sent, the players' names and the server's answer. A file is sent again only once it's grown, so a restart or a lost connection never loses or repeats an upload. Every change to it is saved before the next upload: a save that fails is reported and tried again each poll, and uploads wait until it works. A `uploads.json` that can't be read is never written over: uploads wait, with the error in the window, until it's fixed or deleted. Deleting it costs a round of `duplicate` answers, and starts uploads to each server afresh from then (see `started`).
- A failed upload (offline, server down) isn't in `uploads.json`: the watcher holds it for a backoff and tries again. The window's history (`history.rs`) lists those files first, then `uploads.json`. Its "Retry now" only drops that file's backoff and wakes the uploader, so the file still goes through the same queue and is never sent once it's in `uploads.json` at its size.
- Changing the server, the log folder or the token stops what's left of the uploader's poll (`Uploader::changed`): the next one starts at once with the new settings. Each upload status names the server and folder it was polled for, and the window drops one that isn't about the settings it shows.
- An admin can accept, reject or void a match after its upload, and change a host's trust. Every `STATUS_REFRESH_SECS` (and on "Check again") the uploader asks `GET /api/host/me` and `GET /api/host/matches` for the newest `MAX_STATUS_KEYS` matches, and writes the new status into `uploads.json`, with `matchId`, the match's id on the site (`/match?id=`). The upload's answer has no `matchId`: only this refresh brings it, so the uploader also asks right after an upload that stored a match. The window's "View on the site" appears once it's there. The window shows an untrusted host next to the host name, not in each match's line.
- The window opens a public match's page (`/match?id=`) through `open_match`: it only passes the id, and Rust builds the URL on the current server and opens it with `tauri-plugin-opener`'s Rust API. The plugin isn't registered and the window has no opener permission.
- Each server only gets the logs written after the tool first had a token for it (`started` in `uploads.json`, set when a token for it is first saved, folder or not). So a host who tested on test.genjiball.us and switches to genjiball.us doesn't send the test matches to the real leaderboard. Matches from before that go to a server by hand (genjiball-ranked `npm run upload`) or, for v1.3.2 logs, through the admin's legacy import.
- Credential Manager can't be reached from a non-interactive session (SSH, some agent shells: `cmdkey` fails there too), so the tool uses the DPAPI file there. Test the Credential Manager path from a normal desktop session.

## Contracts with the other repos

- **The log format** is defined in GenjiBall-CE's [`docs/ranked-log.md`](https://github.com/Genji-Ball-Team/GenjiBall-CE/blob/v1.3.3R/docs/ranked-log.md) on the `v1.3.3R` branch. Follow it and never guess past it; if the tool needs a format change, change the spec there first, in its own PR. Its example log (`docs/ranked-log-example.txt`) is the test fixture.
- **The upload API** is defined in genjiball-ranked's [`docs/api.md`](https://github.com/Genji-Ball-Team/genjiball-ranked/blob/main/docs/api.md): the headers to send, which errors to retry and which to stop on.
- **One match, several files:** when the host moves to or from spectator, Overwatch starts a new log file that repeats the match so far. Upload every file once it has a new `MATCH_END` or stops growing (`QUIET_SECS`); the server keeps the longest copy per host + `matchKey`. Only complete lines count and are sent (up to the file's last `\n`, `log_scan::complete_lines`): the game may be halfway through writing a `MATCH_END`, and the server can't repair a match uploaded with half a line. The `[hh:mm:ss]` prefix counts from the game start, so take times from the file name or the clock, not from it.
- **The ranked code** is the latest GenjiBall-CE `R` release with the `RANKS - generated` rule replaced from the rank tags endpoint (`docs/rank-tags.md` on `v1.3.3R`).

## Running it

- Closing the window hides it: the tool keeps uploading from the tray, and quits from the tray menu. Only one copy runs (`tauri-plugin-single-instance`): starting it again shows the running one's window, so two uploaders never share `uploads.json`. A `tauri dev` therefore won't start while the installed tool is running; quit that from the tray first. A `tauri dev` you stop from the shell can leave the app and its `msedgewebview2.exe` processes behind; the next start then fails with "WebView2 error ... requested resource is in use" until they're stopped.
- To test uploads end to end, run genjiball-ranked locally (`npm run dev`) and point the tool at it (Advanced → Server URL `http://127.0.0.1:<port>`) with a folder of test logs. Don't test with the detected folder: it holds the host's real logs, and the default server is the live one.

## PR habits

- One topic per PR, branched from `main`. Fill in `.github/PULL_REQUEST_TEMPLATE.md`, and link the issue (`Fixes #12`).
- Match the surrounding code: `snake_case` in Rust, `camelCase` in TypeScript (serde's `rename_all = "camelCase"` between them), a test for each behaviour.
- Say in the PR what you checked in the running app, since the tests don't open a window.
- Commits are made as `GenjiBallTeam`.

## Other repos

Ranked spans three repos in the Genji-Ball-Team org, cloned side by side in the same parent folder:

| Repo | What it is |
|---|---|
| `GenjiBall-CE` | The game. Ranked logging and the log format spec are on its `v1.3.3R` branch |
| `genjiball-ranked` | Cloudflare Worker: upload API, log parser, ratings, website at genjiball.us (test server: test.genjiball.us) |
| `genjiball-host-tool` (this one) | Tauri app: watches the host's Workshop log folder, uploads matches, builds the ranked code |

- An issue here may need work in another repo. Check that repo's issues before starting, keep one PR per repo, and link them to each other (`Genji-Ball-Team/genjiball-ranked#39`).
- A sibling repo that isn't cloned yet: use `gh -R Genji-Ball-Team/<repo>` rather than guessing its contents. Each repo has its own `AGENTS.md`; follow it when working there.
