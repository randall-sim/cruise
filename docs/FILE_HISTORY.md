# File revision history

Assignments also have a **part slider** above their workspace. The audit saves
the complete displayed file manifest at each learning checkpoint and tracked
operation, reusing history blobs for exact retained bytes. Part labels and
teaching explanations come from the separate `teachingMarkdown` field; raw
checkpoint status is never substituted for a lesson. The student sees one stop
per saved assignment part, followed by Current workspace as the final stop.
Teaching explanations open in a dialog, and every stop uses the same file explorer. File
operations inherit the active part and preserve their own change explanation
in the detailed audit. Shared edits also
appear in affected assignments. Reference renames, removals and ordering preserve
the historical view. Timeline reading is read-only; it never reconciles current
files or restores an old version. Legacy states and unrecorded reasoning are not
backfilled. Teaching-only revisions retain original states and timestamps and
append their own current provenance. See `ASSIGNMENTS.md` for the teaching contract.

The dashboard no longer exposes a file-history viewer or history controls in
course Files, assignment toolbars, or file menus. Existing history and internal
tracking are retained for agent tooling and saved assignment replay. Editor
saves still have an optional Change context field.

Every successful managed create/write/edit is recorded separately, including
same-content saves. Moves preserve the file's identity and timeline; folder
moves/deletes record every eligible child. Removing an assignment reference does
not delete the original or its history. References show the original's timeline,
including edits made by other assignments. Rejected stale saves do not create
revisions. The actor is the calling channel (agent tool, user UI, terminal or
unknown/external), not a cryptographically verified human or model identity.

Agents must use the tracked file tools for **each authored course-file change**,
with a meaningful explanation and optional detailed context describing the task,
source, rubric or observed test result. Never bypass this with direct patches or
shell writes. Read history with `get_file_history`: give courseId and path, plus
assignmentId for assignment paths. Use returned historyId/revisionId for a diff;
offset/limit and diffOffset paginate results. Omitting path lists course histories.

## Coverage

Existing files start with an observed baseline, not a fabricated creation record.
Opening history reconciles the current file with the latest snapshot; outside
changes are labeled unknown author/context. History cannot reconstruct versions
overwritten before tracking began, or multiple external saves between observations.

The command runner snapshots course Files and assignment files before and after
each run, even on failure/cancellation. Entries carry the command, purpose and run
ID. These are observed net changes: intermediate writes within a command are not
captured, and concurrent outside changes cannot reliably be attributed to it.
Failures to checkpoint are surfaced on the command, never silently claimed as
complete coverage. Terminal renames appear as observed deletion/addition; managed
move tools retain lineage. Use file tools for authored changes and commands for
execution/generation. Commands should not mutate history storage.

Dependencies, VCS metadata, build output, credential paths and symlinks are
excluded. Command scanning is bounded to 20,000 entries and 24 directory levels;
the run reports skipped paths. Full snapshots are retained up to 10 MB/file;
larger external files retain metadata only. Binary versions are downloadable but
do not have text diffs. Inline diffs are paginated and bounded; large text versions
remain downloadable, and large changed regions may display as a replacement.

## Storage

History stays in `courses/<id>/history/`: a path/identity index, per-file revision
timelines and deduplicated content snapshots. It persists across app restarts and
belongs with the user's private workspace backups. No automated history pruning
is performed. Snapshot hashes are checked before reading. History is private local
data, not an immutable external audit service; someone with filesystem access
can alter it. Existing workspace content is not migrated by installing this feature.
