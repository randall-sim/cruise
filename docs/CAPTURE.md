# Capture from Chrome

The generic worker connects to a Chrome instance you explicitly prepare. It uses
your already authenticated lecture tab. It does not automate sign-in, copy
cookies, navigate to new pages, click submission buttons, or bypass player access
controls.

## Start a dedicated Chrome profile

Chrome 136+ requires a non-default user data directory for remote debugging.
Use a dedicated course profile rather than your everyday profile. Close its
Chrome window when finished. Never expose the debugging port to the network.

Linux example (Chrome and Course Captain in the same Linux environment):

```sh
google-chrome --remote-debugging-port=9222 \
  --remote-debugging-address=127.0.0.1 \
  --user-data-dir="$PWD/workspace/.runtime/chrome"
```

Windows PowerShell example (run this yourself for the visible sign-in window):

```powershell
& 'C:\Program Files\Google\Chrome\Application\chrome.exe' `
  --remote-debugging-port=9222 `
  --remote-debugging-address=127.0.0.1 `
  "--user-data-dir=$env:LOCALAPPDATA\CourseCaptain\Chrome"
```

For Windows Chrome plus a WSL server, loopback visibility depends on your WSL
networking configuration. Use WSL mirrored networking where available, or run
the worker with Node in the same OS as Chrome. Do not work around this by opening
CDP on `0.0.0.0` or forwarding it publicly. If Windows and WSL use different
checkouts, set `COURSE_CAPTAIN_WORKSPACE` to the same shared workspace. Install
dependencies separately per OS; do not share `node_modules` between them.

Sign in to Canvas and open the actual lecture recording in that profile. Load
the recording, reveal the desired stream layout, and enable captions. Get the
course ID from the UI URL (`?course=...`) or Codex's `list_courses`.

## Discover streams

```sh
npm run capture -- --url "https://your-exact-open-lecture-url" --list
```

The worker lists HTML5 videos in the main document and all browser frames. It
does not assume Panopto, Kaltura, Echo360, Canvas Studio, or Zoom share controls.
A provider can hide a stream behind a switch; reveal it using the player and run
discovery again. Provider-specific switching can be automated by a separately
authorized Codex browser task or a future adapter.

## Use matching course slides when available

Before taking video screenshots, the lecture agent should browse relevant Canvas
modules/pages/files and course website links for the original slides and
instructor notes, following [the lecture agent contract](LECTURE_AGENT.md).
Read those resources and save their text, title, original URL and page/slide
references with `save_course_source` so the guide can use and cite their content.

For a matching deck, prefer readable images rendered/captured from its actual
pages using supported document/browser tools. This avoids seeking the recording
just to obtain a clean picture of every static slide. Inspect the images and
match them to observed lecture anchors; save with `save_lecture_capture`, type
`slide`, a deck/page stream label and provenance in the caption. Its required
timestamp must come from the recording or transcript, never from slide order.
Keep an unmatched page as a linked course resource rather than inventing a time.

The recording still supplies spoken explanations, timing, live annotations,
animation/build states, whiteboards and demos. Capture those separately when
they add information beyond the deck. Note mismatched deck versions and missing
resources. Link the original slides and notes in the guide and identify whether
each image came from a deck or video. The generic capture worker below only
captures video; resource discovery and document rendering are agent steps.

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
record the limitation. Avoid browser chrome, navigation, captions
panels and irrelevant webcam thumbnails. Preserve text and diagram edges.

When slides and a whiteboard both matter, save **two separate screenshots** with
their respective stream/type labels and timestamps. Don't squash them into one
split-screen page capture. Use wider framing only when needed for meaning or when
the tool cannot isolate the region, record that limitation, and restore the original
layout afterward. The generic worker already screenshots each HTML5 video element;
computer-use captures must follow the same framing rule using supported tools.

```sh
npm run capture -- \
  --url "https://your-exact-open-lecture-url" \
  --course COURSE_ID \
  --title "Lecture 05 — Recursion" \
  --date 2026-09-27 \
  --streams 0,1 \
  --interval 3 \
  --kind slide
```

- `--streams` is a comma-separated list from discovery. Omitting it selects all
  discovered HTML5 videos. Streams must be visible and seekable; hidden ones can
  make screenshot capture fail with a partial-coverage result.
- `--interval` is the sampling interval in seconds (1–30; default 3).
- `--kind` is `slide`, `whiteboard`, or `demo`. This is your label for the streams;
  the generic worker does not classify visual content automatically.
- Captions are read from native `textTracks` in the authenticated page. If a
  provider uses custom transcript widgets, download its VTT/SRT and pass
  `--transcript /path/to/captions.vtt`. The worker fails clearly if neither source
  is available. It does not transcribe audio itself.
- `CHROME_CDP_URL` defaults to `http://127.0.0.1:9222` and accepts loopback only.

The worker pauses/seeks videos, samples visible frames, compares reduced grayscale
images, and saves changed frames, the frame before each transition, plus periodic
and final checkpoints. It captures the recording tail even if captions end early.
Each image has a source stream, timestamp, type, and provenance.

It restores timestamps, mute state, caption modes, and playback state when finished
or when an error occurs, and disconnects without closing Chrome. Failed captures
preserve saved images and mark coverage as partial. Recordings over four hours
and more than 1,000 captures per lecture are rejected explicitly.

After capture, the dedicated lecture agent inspects every screenshot through
`read_capture`, adds missing diagrams/demos with `save_lecture_capture`, updates
coverage with `set_lecture_coverage`, and creates/completes the lecture guide job.
Check whether a person can actually read the teaching content. For blur, distant
boards, distorted text or tiny labels, follow
[visual reconstruction instructions](VISUAL_RECONSTRUCTIONS.md): preserve the
original and attach a supported, labeled image or diagram beside it.
Queue the job after captures are saved so its evidence snapshot includes them.
Capture or transcript import alone never completes a user-requested lecture import.
Follow [the lecture agent contract](LECTURE_AGENT.md) for the full workflow.

## What the adapter cannot promise

Sampling and image differences are candidate detection, not proof that every
slide or final diagram was preserved. Animation, tiny writing changes,
transitions, off-screen whiteboards, and fast demonstrations can be missed.
Cross-origin provider APIs, encrypted recordings, and custom media surfaces need
provider-specific support. The lecture page preserves coverage warnings.

The integration test uses an actual six-second WebM, two visible HTML5 streams
(one inside an iframe), native captions, and a local Chromium debugging session.
Live institutional lecture systems require separate verification.

The reader's transcript buttons preserve the recording URL and, for Panopto
Viewer/Embed links, set `start` to the cue's seconds. See Panopto's
[start-at link guidance](https://community.panopto.com/discussion/1243/make-chapters-anchored-and-linkable).
For other players, the reader opens the original recording and shows the time to
seek to; it does not assume an undocumented player parameter.
