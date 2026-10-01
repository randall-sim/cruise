# Local storage and independent repositories

The daemon application owns src/, scripts/, docs/, AGENTS.md, Cargo and Node
manifests, and tests. It ignores the entire workspace/ directory.

The default content root is daemon/workspace/content/, a separate private Git
repository containing the unchanged state.json, courses/, Markdown, captures,
file history and course files. COURSE_CAPTAIN_WORKSPACE overrides that root;
relative values resolve against the daemon checkout. HTTP, MCP and capture must
use the same root. No workspace or credentials belong in the frontend repository.

For a new installation, clone your private user-content repository into
workspace/content, or initialize it there. Content publishing uses that repo's
own tracked remote. Never push coursework to an instructor or submission repo.
Credentials, browser sessions, .runtime, .cache, and .secrets remain ignored.
No remote, automatic sync or GitHub upload is created by starting the daemon.

The Rust process serves the local API; its private Node worker runs existing
course logic. The browser fetches data and assets directly from loopback using
an explicit allowed frontend origin and a connection key. Vercel serves only
the frontend. Server-side rendering must never try to read this local workspace.

Before migration or an application upgrade, stop course writers and back up the
whole content root. Never reset, delete, migrate or clean ignored user data during
an application update. The split keeps an original checkout as a recovery copy;
it does not rewrite old Git history. New content keeps the existing version-1
schema, atomic writes, process-shared filesystem locks and course-relative paths.

Editing generated notes directly does not update structured guide sections in
state.json. Keep additional notes in memory or rebuild through course tools.
Multi-file writes save companion files before committing metadata. A crash can
leave an orphan companion file; back up the entire content repo, not just the index.
