# Course Captain daemon (`cc-daemon`)

The local backend owns course storage, agent instructions, MCP, capture and local
course execution. The browser talks to the Rust HTTP process directly; no course
data is sent through Vercel.

This migration uses a Rust API/CLI with the existing TypeScript course engine in
a private Node worker. The engine and MCP have **not** been rewritten in Rust.
The worker communicates over stdin/stdout, with no separate HTTP server.
Worker exchanges finish even when the browser disconnects, so refreshing cannot
leave a reply for the next request. Capture images and reconstruction artifacts
are served directly by Rust under the same authentication and origin checks.
A small in-memory path index refreshes when `state.json` changes; image reads run
independently of the course worker and do not cross its JSON/base64 transport.

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

The file viewer's **Open in file explorer** button sends an authenticated POST
to the daemon. Windows reveals the file in Explorer, macOS reveals it in Finder,
and Linux opens its containing folder with `xdg-open` (requires a desktop file
manager). WSL converts the path with `wslpath` and launches Windows Explorer.
Saved steps reveal the original path on disk; if it is gone, the nearest existing
course-files folder opens. Restart the daemon after updating its API worker.

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
`package.json`, and `scripts/`. Use this folder for course work rather than the
frontend or the nested content repository, so Codex loads the daemon's agent
instructions and project tools.

After installing dependencies as described above, run these commands from the
daemon folder on the **Codex host OS**:

```sh
node scripts/setup-codex.mjs
node scripts/check-mcp.mjs
```

For Windows Codex opening a WSL UNC path, run the setup command on Windows; the
generated launcher runs the course engine inside Ubuntu, where you installed
the dependencies. Run the check command on Windows too. For a native Linux or
macOS Codex host, run both commands there.

Reopen the project in Codex to load MCP, and approve project trust in Codex if
prompted. The check command verifies that the tools connect and the workspace
is readable. MCP does not require the HTTP server or frontend to be running;
start `course-captain run` when you want to use the browser dashboard.

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
cargo build
npm run typecheck
npm test
npm run build
npm run check:mcp
```

Browser access to localhost from HTTPS is subject to browser permissions. Allow
access to apps on this device when prompted. This is a direct local connection;
there is no Vercel proxy, tunnel, or cloud copy of course data.
