# Generate a course practice exam

Never submit anything to official course submission destinations (including Canvas, Gradescope, autograders, submission email, repositories and CLI/API endpoints), through any tool or delegated agent. Keep all generated output inside the course workspace, including code, answers, artifacts and command results; return workspace links and status. Only the private GitHub workspace backup authorized in AGENTS.md is excepted; never push to a course submission repository.

Use this workflow when the user asks for an exam, mock exam, practice test or
exam-style question set. The user's Codex session creates the exam; the dashboard
is a reader. Do the whole workflow, not just preparation or queueing.

## Discover the format and resolve coverage

1. Call `prepare_exam` with the course ID and the user's original prompt. It
   returns course links, the first 50 lecture catalog entries and ranked local
   evidence about exams. Use `list_lectures` for remaining catalog pages and
   `search_course` for targeted searches of Markdown notes and lecture evidence.
   Search for exam format, midterm dates/scope, past/practice papers, rubrics,
   instructor guidance, permitted aids and topic weightings.
2. Read the applicable computer-use skill and browse the course website and
   Canvas. Follow syllabus, exam, module, announcement, file and practice-paper
   links; open/read the actual materials. Check relevant saved course notes for extra
   context, preserving the official-source priority in `TASK_WORKFLOW.md`. A
   lecture statement or instructor note may supply the format even when no
   practice paper exists. Save relevant excerpts with `save_course_source`,
   original URLs, page references and source dates, then retrieve their IDs with
   `search_course`. State which resources you actually inspected and any access
   gaps. Never claim an unseen paper was reviewed.
3. Translate the requested timeline into one supported scope:
   - **Dates:** `mode: "dates"`, `from` and `through` as real YYYY-MM-DD dates,
     inclusive. Resolve month names and year from the course term and request.
     For example, October through November 2026 is 2026-10-01 to 2026-11-30.
   - **Lecture range:** `mode: "lectures"`, with the actual catalog `lectureIds`.
     Resolve “lectures 1–8” by their titles/course schedule, not array position or
     the first eight imported files. Report missing lectures or numbering gaps;
     do not substitute adjacent lectures. Date ordering is not lecture numbering.
   - **After an event:** `mode: "after_event"`, `event`, verified `date`, an upper
     `through` date, and `evidenceIds` supporting the event cutoff. For “after the
     last midterm,” identify the most recent completed relevant midterm relative
     to the requested end/current date, not a future scheduled exam. Check whether
     the user means the exam date or its topic-coverage boundary. This mode selects
     dates strictly after the event day. For same-day lecture boundaries or a
     topic cutoff, use explicit lecture IDs and explain the supporting evidence.
     Inspect actual sources to resolve ambiguity. Ask only when material uncertainty
     remains; do not invent a date or silently broaden the scope. If necessary
     lectures are absent, report them and ingest them only within the user's scope.
4. Distinguish **content sources** from **format sources**. All transcripts and
   images belonging to selected lectures enter the evidence snapshot. Use
   `contentSourceIds` for relevant saved Markdown/source excerpts, checking their
   topics against the requested scope; Markdown files have no reliable lecture
   date by default. A newer file import timestamp is not its teaching date.
   `formatSourceIds` may come from outside the topic range but justify only the
   question types, instructions and marking style, not out-of-scope questions.
   Event evidence similarly justifies the cutoff. Keep practice-paper solutions
   out of content unless they genuinely support an in-scope concept.
5. Call `preview_exam_scope` and inspect the selected lecture titles/dates and
   source roles. Tell the user the resolved coverage as a progress update; no
   extra approval is needed when it follows their request. Choose question count,
   types and duration from the user's preferences and verified format sources.
   Supported types: `multiple_choice`, `multiple_select`, `true_false`,
   `short_answer`, `essay`, `calculation`, `proof`, `code`, `diagram`.
   If no format is evidenced, explicitly propose an inferred practice mix,
   defaulting to 12 questions unless another size is appropriate. The tool supports
   1–60 questions, and an optional duration of 1–360 minutes. Do not claim these
   defaults match the instructor's real exam. User-requested adaptations must be
   labeled even when the underlying format is sourced.

## Create and save the exam

6. Call `queue_exam` with the resolved request. This snapshots course evidence and
   queues an `exam` job. Then read `get_job` through `nextOffset = null` in bounded
   pages. Each page's `contentEvidenceIds` lists the IDs eligible for question and
   answer support; format/event IDs are separate in `examRequest`. Inspect images
   through `read_capture` before relying on their content. Generated guides can
   orient you, but original sources should ground claims. Do not treat lexical
   retrieval or a valid citation ID as proof that a claim is supported.
7. Build original questions across the selected topics with a suitable difficulty
   progression. Practice papers inform structure and difficulty, not copied
   questions or solutions. Use fresh wording, values and scenarios. Balance the
   scope; identify what the set does not test. Include the requested count and
   every explicitly requested question type. Code is inert Markdown, never run.
8. Use the exam output schema from `get_job`: title, instructions, format rationale
   with citations and `inferred`, questions, and gaps. Every question needs points,
   a topic, prompt, an answer, explanatory solution, point-based rubric and content
   citations. Rubric points must sum to question points. Choice IDs must be unique
   and correct IDs must exist. Single-choice and true/false have one correct ID;
   multiple-select has at least two. True/false choices are labeled True and False.
   Written questions use empty choice arrays. Do not leak solutions into prompts,
   choices or general instructions. Put source citations with the answer so the
   student can attempt the question without hints.
9. `extend_job_evidence` cannot broaden a queued exam's frozen scope. If essential
   evidence is missing, save it, fail the obsolete job with a clear reason, and
   queue a replacement with the corrected sources. Do not fabricate citations or
   append unrelated material. `complete_job` validates counts, types, answer
   choices, rubrics, source roles and scope, then stores `exams/<job-id>/exam.md`
   and `answer-key.md` plus structured UI data. Unrepresented lectures are flagged
   as coverage gaps. These checks do not validate pedagogical quality or semantic
   correctness; review every solution and source yourself before completing.
10. Verify the completed job with `get_job`. Report the saved practice exam and
    its scope, question mix and significant gaps. It appears under the course's
    **Exams** tab. Answers, explanations, marking guides and evidence stay hidden
    in reveal panels until the user opens them. This is self-study material, not
    a forecast of the actual exam or a guarantee of comprehensive preparation.

Examples: “Create a 20-question exam from lectures 1–8”; “Make a practice final
covering October through November”; “Make a 60-minute mock exam on everything
after the last midterm, following the practice-paper format.”

## Assignment learning as exam evidence

Exams can use the assignment RAG/Markdown memory in `memory/assignments/`.
`prepare_exam` returns relevant `assignmentEvidence`; retrieve more with
`search_course`, then read the learning notes and supporting course sources.
Choose excerpts that actually match the user's date/lecture/event scope and
include their IDs in `contentSourceIds`. `preview_exam_scope` reports selected
assignment evidence; `queue_exam` snapshots it with other content evidence.
Questions, explanations and rubrics can cite these IDs. Generated assignment
reasoning is secondary evidence and may be mistaken: cross-check it rather than
treating it as instructor truth, and retain stated uncertainties.

Shared course code/resources in the Files tab and observed command records in
`memory/files/` are also searchable exam evidence. Select relevant source IDs in
`contentSourceIds` after checking the requested scope; a file or test passing is
not automatically instructor authority or evidence of broader correctness.
