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
- Unfinished features ship behind a flag, off by default.
- Debug settings come with the features (#11): a log level, a dry run that shows what would be uploaded, a debug panel and a diagnostics export.

## The host token

It's kept in the OS credential store, never in a settings file, a log, an error message or a diagnostics export. Settings files hold everything else.

## Contracts with the other repos

- **The log format** is defined in GenjiBall-CE's [`docs/ranked-log.md`](https://github.com/Genji-Ball-Team/GenjiBall-CE/blob/v1.3.3R/docs/ranked-log.md) on the `v1.3.3R` branch. Follow it and never guess past it; if the tool needs a format change, change the spec there first, in its own PR. Its example log (`docs/ranked-log-example.txt`) is the test fixture.
- **The upload API** is defined in genjiball-ranked's [`docs/api.md`](https://github.com/Genji-Ball-Team/genjiball-ranked/blob/main/docs/api.md): the headers to send, which errors to retry and which to stop on.
- **One match, several files:** when the host moves to or from spectator, Overwatch starts a new log file that repeats the match so far. Upload every file as it grows; the server keeps the longest copy per host + `matchKey`. The `[hh:mm:ss]` prefix counts from the game start, so take times from the file name or the clock, not from it.
- **The ranked code** is the latest GenjiBall-CE `R` release with the `RANKS - generated` rule replaced from the rank tags endpoint (`docs/rank-tags.md` on `v1.3.3R`).

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
