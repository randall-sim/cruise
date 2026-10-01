# Course Captain daemon (`cc-daemon`)

The local backend owns course storage, agent instructions, MCP, capture and local
course execution. The browser talks to the Rust HTTP process directly; no course
data is sent through Vercel.

This migration uses a Rust API/CLI with the existing TypeScript course engine in
a private Node worker. The engine and MCP have **not** been rewritten in Rust.
The worker communicates over stdin/stdout, with no separate HTTP server.

## Install and run

Requires Rust and Node.js 22 or newer. In this directory:

```sh
npm ci
npm run build
npm link
course-captain run
```

For the deployed frontend, allow its exact origin:

```sh
course-captain run --origin https://your-app.vercel.app
```

Open the frontend and enter the connection key printed in the terminal. The key
changes when the process restarts. Ctrl+C stops the server and its worker.
Optional `--port 4321` and `--workspace /absolute/path` override local defaults.
Origins can also be comma-separated in `COURSE_CAPTAIN_ALLOWED_ORIGINS` in
`.env.local`. HTTPS origins and HTTP loopback origins are supported; wildcards
and URL paths are rejected. Preview deployments need their own explicit origin.
The server binds only to `127.0.0.1` and verifies the Host header as well as Origin.

If your global npm directory requires administrator access, use
`npm_config_prefix="$HOME/.local" npm link` and add `$HOME/.local/bin` to PATH.
If this checkout moves after installation, rebuild and install the launcher again.
For Windows with a WSL checkout, build in Ubuntu. The Node launcher detects WSL;
`node scripts/cli.mjs run` starts that binary from Windows. On this machine a
per-user `course-captain` launcher is already registered in Windows and Ubuntu;
open a new terminal to refresh PATH. Native Windows checkouts can use `npm link`.
Run `cargo run -- run` during development instead of installing the command.

## Private workspace

```text
daemon/                   # cc-daemon application Git repository
  AGENTS.md
  docs/
  workspace/              # entirely ignored by application Git
    content/              # independent private user-content Git repository
      state.json
      courses/
```

The default storage root is `workspace/content`. The existing state format and
course-relative paths are unchanged. Clone a private content repository into
that directory on a new installation, or initialize it with `git init`.
Never put credentials, browser sessions or derived caches in the content repo.
Application upgrades must not reset or clean the ignored workspace.

This extracted checkout includes a copy of the existing workspace. Its original
copy is retained by the parent migration backup. This migration configured a
separate private content remote; the daemon itself never creates remotes or
pushes on startup. No course repository is ever a submission destination.

## Codex

Open this daemon directory in Codex. On the **Codex host OS**, run:

```sh
node scripts/setup-codex.mjs
```

For Windows Codex opening a WSL UNC path, run the setup command on Windows; the
generated launcher runs the course engine inside Ubuntu. Reopen the project to
load MCP. MCP does not require the HTTP server or frontend to be running.

## Checks

```sh
cargo test
cargo build
npm run typecheck
npm test
npm run build
npm run check:mcp
```

Browser access to localhost from HTTPS is subject to browser permissions. Allow
access to apps on this device when prompted. This is a direct local connection;
there is no Vercel proxy, tunnel, or cloud copy of course data.
