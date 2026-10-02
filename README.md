# cruise daemon (`cc-daemon`)

The local backend owns course storage, agent instructions, MCP, capture and local
course execution. The browser talks to the Rust HTTP process directly; no course
data is sent through Vercel.

The CLI, HTTP API, course engine, file history, local command runner and MCP are
implemented in Rust in one executable. Running the backend requires no Node.js,
npm packages or separate worker. Capture images and reconstruction artifacts
use the same authentication and origin checks as the API.

## Install and run

Requires the Rust toolchain to build. In this directory:

```sh
cargo install --path . --force
cruise run
```

For the deployed frontend, allow its exact origin:

```sh
cruise run --origin https://cruise.ink
```

Open the frontend and enter the connection key printed in the terminal. The key
stays the same across page refreshes and daemon restarts. It is stored in
`$CRUISE_HOME/workspace/connection-key` (the daemon checkout by default), outside
`workspace/content`, with owner-only permissions on Unix. Ctrl+C stops the server.
Run `cruise reset-connection-key` or use the local MCP tool `reset_connection_key`
if the key is compromised. Reset immediately rejects the old key on running
servers; reconnect browsers using the new key. The reset tool is only exposed
locally through CLI/MCP, not over the HTTP API.
Optional `--port 4321` and `--workspace /absolute/path` override local defaults.
Origins can also be comma-separated in `CRUISE_ALLOWED_ORIGINS` in
`.env.local`. HTTPS origins and HTTP loopback origins are supported; wildcards
and URL paths are rejected. Preview deployments need their own explicit origin.
The server binds only to `127.0.0.1` and verifies the Host header as well as Origin.
Existing `COURSE_CAPTAIN_*` environment settings remain supported.

Ensure Cargo's bin directory (`~/.cargo/bin`) is on PATH. Alternatively,
`sh scripts/install.sh` installs into `~/.local/bin` on Linux/macOS. For Windows, run
`powershell -ExecutionPolicy Bypass -File scripts/install.ps1` from this directory.
For a WSL checkout, that script builds in Ubuntu and installs a per-user Windows
launcher that invokes the Rust executable through `wsl.exe`. Native Windows
checkouts install a native Rust executable. Open a new terminal to refresh PATH.
If this checkout moves, rebuild and reinstall. During development, use
`cargo run -- run` instead of installing the command.

`cruise capture --help` describes the native lecture screenshot sampler for an
already-open Chrome debugging session; see [capture instructions](docs/CAPTURE.md).
To explicitly create fictional demo content in an empty workspace, run
`cruise demo --workspace /path/to/empty/workspace`.

The file viewer's **Open in file explorer** button sends an authenticated POST
to the daemon. Windows reveals the file in Explorer, macOS reveals it in Finder,
and Linux opens its containing folder with `xdg-open` (requires a desktop file
manager). WSL converts the path with `wslpath` and launches Windows Explorer.
Saved steps reveal the original path on disk; if it is gone, the nearest existing
course-files folder opens. Rebuild and restart the daemon after updating it.

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

The default storage root is `workspace/content`. The engine creates this folder
when first accessed and creates state, course folders, course `AGENTS.md` files,
and memory as you add content. You do not need to create the folder structure
manually, and Git is optional for local use.

To restore existing content, clone your private content repository into that
directory before adding courses. For a new Git backup, initialize a separate
repository there, add a content `.gitignore`, and configure your private remote.
These Git setup steps are not automatic; the daemon's `/workspace/` ignore rule
does not protect files committed from inside the separate content repository.
Never put credentials, browser sessions or derived caches in the content repo.
Application upgrades must not reset or clean the ignored workspace.

The daemon itself never creates remotes or pushes on startup. Once you configure
an authorized private backup remote, Codex follows `AGENTS.md` to commit and push
completed course-content changes. Instructor and course submission repositories
must never be used as backup destinations.

## Codex

**Open the daemon repository folder as your project in Codex.** In the split
checkout, select `course-captain/daemon`; if you cloned `cc-daemon` by itself,
select that clone's root. This is the folder containing `AGENTS.md`, `Cargo.toml`,
and `scripts/`. Use this folder for course work rather than the
frontend or the nested content repository, so Codex loads the daemon's agent
instructions and project tools.

After installing the executable, run these commands from the daemon folder:

```sh
cruise setup-codex
cruise check-mcp
```

For Windows Codex opening a WSL UNC path, use `cruise setup-codex --windows-host`
inside WSL (or through the installed Windows launcher). It registers `wsl.exe`
with the exact Rust executable and checkout paths. For native Linux, macOS or
Windows Codex, omit `--windows-host`. Setup only updates its marked block in the
project config; it never changes global settings or trust.

Reopen the project in Codex to load MCP, and approve project trust in Codex if
prompted. The check command verifies that the tools connect and the workspace
is readable. MCP does not require the HTTP server or frontend to be running;
start `cruise run` when you want to use the browser dashboard.

Ask Codex for course work in ordinary language. It uses the MCP tools to save
results in your workspace. Lecture-guide creation currently requires a dedicated
GPT-6 Astra worker at medium reasoning, as specified in
[the lecture workflow](docs/LECTURE_AGENT.md). Reading recordings and official
course pages also requires browser/computer-use tools and your authenticated
course-site session; you handle any login or MFA prompts.

### Example requests

Replace course names, dates, and links with your own. These prompts describe
supported workflows; opening the project alone does not start any of them.

| Use case | Example prompt |
| --- | --- |
| Set up a course | "Add CS 101, Introduction to Computer Science, for Fall 2026. Its course website is [course URL] and its Canvas page is [Canvas URL]." |
| Create a lecture guide | "Import the CS 101 lecture from October 1, 2026 at [recording URL]. Create the complete guide with slides, Detailed and Fast explanations, Word bank terms, and practice questions." |
| Explain a difficult topic | "Explain recursion using my saved CS 101 lectures. Walk through an example and cite the relevant lecture timestamps." |
| Check upcoming work | "Check the official CS 101 course pages for assignments due next week. Compare them with my saved progress and tell me what remains." |
| Work through an assignment | "Help me work through CS 101 Assignment 2 one part at a time. Read the official requirements, use the course's methods, explain each step, and run local checks. Save progress without submitting anything." |
| Resume saved work | "Resume CS 101 Assignment 2 from its latest saved checkpoint. Explain where I left off and continue with the next part." |
| Prepare for an exam | "Create a CS 101 practice exam covering lectures 1–8. Check the instructor's exam format, flag missing material, and include worked answers." |
| Back up course content | "Set up Git backup for my workspace content using my private repository [private backup repository URL]. Exclude credentials, browser sessions, caches, and generated dependencies, then commit and push the course content." |

Generated guides, files, and learning notes stay in the local course workspace.
The agent does not submit assignments, take official quizzes, or change grades.
See [the agent workflow](docs/AGENT_WORKFLOW.md) for the full contract.

## Checks

```sh
cargo test
cargo build --release
cruise check-mcp
```

Browser access to localhost from HTTPS is subject to browser permissions. Allow
access to apps on this device when prompted. This is a direct local connection;
there is no Vercel proxy, tunnel, or cloud copy of course data.
