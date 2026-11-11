# Termy XL

Termy XL is the standalone Trends Desk: Law, Finance, AI, and custom columns. The window is a normal GPUI surface. Searches run on a background thread through OpenCLI. The GPU draws the window. It does not make the search faster.

Termy X stays a separate app. Its shortcut, installer, and process name are unchanged, including a shortcut that launches `termy-7154b8a5.exe`. Termy XL is also opened from inside Termy (command palette, View menu, Trends tab) as the same window. Its settings, history, and drafts stay in the Termy XL data folder (`%APPDATA%\Termy XL`, or `TERMY_XL_DATA_DIR`). They are not written into Termy X AppData unless you use **Copy draft to Termy X** or `termy-xcli trends-desk handoff`.

The browser desk at `http://127.0.0.1:3489/` is a separate local app. This add-on does not start it, proxy it, or require it.

## What you can do

- Run one column or every column.
- Keep a focus term in the search when the column text does not already contain it.
- See fresh, cached, empty, error, and loading states as different states.
- Open a source link, inspect replies, and prepare a draft.
- Read saved reports. A failed or empty run does not write a new one, and it does not label an old report as fresh.

Run, inspect, and draft preparation do not publish. A draft is stored in the existing Termy X draft file. Sending still goes through X compose, or `termy-xcli publish <id>`, and still requires you to confirm. If saving the draft file fails, the text stays in the Draft surface.

## Open it

Inside Termy X (the existing app):

- Command palette: **Termy XL** (`open_trends_desk`)
- View menu: **Termy XL**
- X panel, Trends tab: **Open Termy XL**

On its own:

```sh
cargo run -p termy --bin trends_desk
termy --trends-desk
```

The requested window size is 3440×1400. Narrower windows drop from four columns to three, two, then one.

Labeled sample cards, with no OpenCLI call and no saved report:

```sh
cargo run -p termy --bin trends_desk -- --fixture
```

Headless, same rules:

```sh
termy-xcli trends-desk status
termy-xcli trends-desk run --topic law --focus AI
termy-xcli trends-desk run --topic all
termy-xcli trends-desk add "Local" "notes that stay in the query"
```

The program file for the packaged CLI is `termy-xcli` (`termy-xcli.exe` on Windows). Do not run a file named `x.exe`.

## Live data

A live column needs OpenCLI (`@jackwener/opencli`) on `PATH`, or `OPENCLI_HERMES_NODE_DIR` pointing at the directory that contains `node` / `node.exe` and `node_modules/@jackwener/opencli`. The desk uses the same command runner as the rest of Termy X: argument vector, no shell, Windows children without a console flash.

There is no fallback to sample posts, and the paid X API is not called. Image-only quote posts are not drawn here. Photos and video stay in the X panel. VLC is unchanged.

Details: [API_CONTRACT.md](API_CONTRACT.md), [LAUNCH_AND_DEPLOYMENT.md](LAUNCH_AND_DEPLOYMENT.md), [SANITIZATION.md](SANITIZATION.md).
