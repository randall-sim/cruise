# Capture from Chrome

Use the authenticated course tab through the session's browser/computer-use tools.
The Rust daemon stores transcripts and images through MCP. The optional native
`cruise capture` command samples already-open HTML5 recordings through Chrome's
loopback debugging endpoint. Follow the browser tool's instructions,
leave sign-in and MFA to the user, and never copy cookies or bypass player access
controls. Never use course submission controls.

## Native automatic sampler

Use a dedicated Chrome profile with remote debugging enabled on loopback. Sign
in and open the lecture yourself. The sampler connects to that browser, selects
the exact existing tab URL, and disconnects when finished; it never launches a
browser or signs in. `CHROME_CDP_URL` defaults to `http://127.0.0.1:9222` and must
be a loopback endpoint.

Inspect the accessible HTML5 streams before choosing them:

```sh
cruise capture --url "https://your-recording-url" --list
```

Then capture the selected streams into an existing course:

```sh
cruise capture --url "https://your-recording-url" --course COURSE_ID --title "Lecture title" --streams 0,1 --transcript /path/to/transcript.vtt
```

If `--transcript` is omitted, the sampler tries the selected videos' native
caption tracks. It requires a real transcript before creating a lecture. The
optional `--date YYYY-MM-DD`, `--interval 3`, `--kind slide`, and `--workspace PATH`
control the lecture date, sample spacing, capture type and storage location.
Intervals range from 1 to 30 seconds; types are `slide`, `whiteboard`, and `demo`.
Omitting `--streams` samples every discovered stream. Recordings must be seekable
and no longer than four hours.

The sampler saves visible changes, the frame before a transition, periodic
checkpoints and the recording tail. It records a capture manifest and coverage
limitations, and restores each video's time, mute, playback and caption settings
on completion or failure. A failure after import marks the lecture as partial.
Review the saved images and any missing streams before generating a guide.
Provider-specific players without accessible HTML5 streams require the browser
workflow below. The sampler requires no Node.js runtime.

## Discover streams and transcripts

Inspect the recording's visible controls and streams. A provider can hide a stream
behind a switch; reveal it through the player and inspect again. Do not assume
Panopto, Kaltura, Echo360, Canvas Studio or Zoom share controls. Record which streams
were actually inspected and any unavailable material.

Get the course ID from `list_courses`. Obtain the recording's real VTT/SRT through
its supported transcript/download controls and pass the text, recording URL,
lecture title and date to `import_lecture`. If the provider exposes only a custom
transcript, use the session's supported browser workflow to obtain it; never
invent timestamps or claim a transcript was read when it was inaccessible.
Transcript ingestion alone does not complete a lecture import.

## Use matching course slides when available

Before taking video screenshots, browse the lecture's relevant Canvas modules,
pages and files and the course website for original slides and instructor notes,
following [the lecture agent contract](LECTURE_AGENT.md). Read those resources
and save their text, title, original URL and page/slide references with
`save_course_source` so the guide can use and cite their content.

For a matching deck, prefer readable images rendered/captured from its actual
pages using supported document/browser tools. Inspect the images and match them
to observed lecture anchors. Save with `save_lecture_capture`, `kind=slide`, a
deck/page stream label and provenance in the caption. Its required timestamp
must come from the recording or transcript, never from slide order. Keep an
unmatched page as a linked course resource rather than inventing a time.

The recording still supplies spoken explanations, timing, live annotations,
animation/build states, whiteboards and demos. Capture those separately when
they add information beyond the deck. Note mismatched deck versions and missing
resources. Link the original slides and notes in the guide and identify whether
each image came from a deck or video.

## Capture

Capture the teaching content itself, not the full lecture viewer page. Prefer a
video/slide element screenshot or a tightly framed region containing the complete
slide, board or demonstration. **Enlarge before capturing:** inspect the site's
actual player controls and try fullscreen or expand/maximize for the selected
stream first. If unavailable or unsuccessful, use the site's theater, single-stream
or largest layout and collapse adjacent panels through site controls where possible.
Make the teaching video as large as the available screen permits before every
screenshot; repeat this check after switching streams or layouts. Wait for the
enlarged frame to finish rendering and controls/overlays to disappear, then capture.
Cropping a small player or enlarging the saved image does not improve its captured
resolution. If site controls cannot enlarge it, capture the best available view and
record the limitation. Avoid browser chrome, navigation, captions panels and
irrelevant webcam thumbnails. Preserve text and diagram edges.

When slides and a whiteboard both matter, save **two separate screenshots** with
their respective stream/type labels and verified timestamps. Do not squash them
into one split-screen page capture. Use wider framing only when needed for meaning
or when the tool cannot isolate the region; record that limitation.

Save each image through `save_lecture_capture` with the lecture ID, observed
`seconds`, `stream`, `kind` (`slide`, `whiteboard` or `demo`), caption/provenance
and a PNG, JPEG or WebP data URI. The Rust daemon validates and normalizes the
image and saves it in the course workspace. Capture final whiteboard states,
worked examples, transitions and the recording tail even if captions end early.
Inspect actual image readability, not just dimensions. Restore the recording's
original layout, time, mute and playback state when finished or on failure.

Use `set_lecture_coverage` to record actual inspected streams, time coverage and
concrete limitations. A partial capture remains partial; do not describe it as
complete merely because screenshots were saved.

After capture, the dedicated lecture agent queues the lecture job, inspects every
screenshot through `read_capture`, and reads every evidence page. Add missing
diagrams/demos with `save_lecture_capture`; queue a new job after new captures so
its evidence snapshot includes them. For blur, distant boards, distorted text or
tiny labels, follow [visual reconstruction instructions](VISUAL_RECONSTRUCTIONS.md):
preserve the original and attach a supported, labeled image or diagram beside it.
Complete the guide through `complete_job`. Capture or transcript import alone
never completes a user-requested lecture import. Follow
[the lecture agent contract](LECTURE_AGENT.md) for the full workflow.

## Coverage limits

Saved images alone do not prove every slide or final diagram was preserved.
Animation, tiny writing changes, transitions, off-screen whiteboards and fast
demonstrations can be missed. Cross-origin provider controls, encrypted recordings
and custom media surfaces can limit the browser tools. State the actual gaps.

The reader's transcript buttons preserve the recording URL and, for Panopto
Viewer/Embed links, set `start` to the cue's seconds. See Panopto's
[start-at link guidance](https://community.panopto.com/discussion/1243/make-chapters-anchored-and-linkable).
For other players, the reader opens the original recording and shows the time to
seek to; it does not assume an undocumented player parameter.
