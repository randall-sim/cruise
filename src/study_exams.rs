use super::*;

pub fn prepare(store: &Store, args: &Value) -> Result<Value> {
    let course_id = arg(args, "courseId")?;
    let prompt = arg(args, "prompt")?;
    let state = store.read()?;
    let course = require_course(&state, course_id)?;
    let sources = evidence(store, &state, course_id)?;
    let mut lectures: Vec<_> = list(&state, "lectures")
        .iter()
        .filter(|l| text(l, "courseId") == course_id)
        .cloned()
        .collect();
    lectures.sort_by(|a, b| text(a, "date").cmp(text(b, "date")));
    let assignment_sources = sources
        .iter()
        .filter(|s| {
            text(s, "kind") == "assignment"
                && text(s, "path").starts_with(&format!("courses/{course_id}/memory/assignments/"))
        })
        .cloned()
        .collect();
    Ok(
        json!({"status":"discovery_required","course":course,"prompt":prompt,"instructions":EXAM_POLICY,"lectures":lectures.iter().take(50).map(|l|json!({"id":l["id"],"title":l["title"],"date":l["date"]})).collect::<Vec<_>>(),"totalLectures":lectures.len(),"assignmentEvidence":rank_evidence(assignment_sources,prompt,12),"assignmentMemoryInstructions":"Read relevant assignment learning records with search_course and the course context reader. Select in-scope excerpts explicitly in contentSourceIds. These are generated explanations/work history, not instructor authority; cross-check concepts against the cited course sources. Do not assume all assignment topics fall within the exam's date/lecture scope.","nextStep":"Use list_lectures for additional catalog pages; search_course for more scope/format evidence. Inspect browser resources, save relevant sources, resolve coverage, then preview_exam_scope and queue_exam. This tool has not created an exam.","evidence":rank_evidence(sources,&format!("exam midterm final practice sample format rubric syllabus {prompt}"),20)}),
    )
}

fn require_ids(ids: &[Value], sources: &[Value], label: &str) -> Result<()> {
    if ids.iter().any(|id| !sources.iter().any(|s| s["id"] == *id)) {
        return Err(format!(
            "{label} contains unavailable or other-course evidence"
        ));
    }
    Ok(())
}
fn resolve(store: &Store, state: &Value, args: Value) -> Result<(Value, Vec<Value>, Vec<Value>)> {
    let mut data = crate::mcp::validate_input("queue_exam", args)?;
    let course_id = arg(&data, "courseId")?.to_owned();
    require_course(state, &course_id)?;
    let prompt = arg(&data, "prompt")?.trim().to_owned();
    if prompt.is_empty() {
        return Err("Prompt cannot be empty".into());
    }
    data["prompt"] = json!(prompt);
    let mut lectures: Vec<_> = list(state, "lectures")
        .iter()
        .filter(|l| text(l, "courseId") == course_id)
        .cloned()
        .collect();
    lectures.sort_by(|a, b| text(a, "date").cmp(text(b, "date")));
    let scope = &data["scope"];
    let mode = text(scope, "mode");
    if mode != "lectures" {
        let first = if mode == "dates" { "from" } else { "date" };
        for key in [first, "through"] {
            chrono::NaiveDate::parse_from_str(text(scope, key), "%Y-%m-%d")
                .map_err(|_| format!("Invalid exam date: {key}"))?;
        }
        if text(scope, first) > text(scope, "through") {
            return Err("Exam date range is reversed".into());
        }
    }
    let types = strings(&data, "questionTypes");
    if types.iter().collect::<HashSet<_>>().len() != types.len()
        || types.len() > data["questionCount"].as_u64().unwrap_or(12) as usize
    {
        return Err("Question types must be unique and fit the question count".into());
    }
    if mode == "lectures"
        && list(scope, "lectureIds")
            .iter()
            .any(|id| !lectures.iter().any(|l| l["id"] == *id))
    {
        return Err("A selected lecture is not in this course".into());
    }
    let selected: Vec<_> = lectures
        .into_iter()
        .filter(|l| match mode {
            "lectures" => list(scope, "lectureIds").contains(&l["id"]),
            "dates" => {
                text(l, "date") >= text(scope, "from") && text(l, "date") <= text(scope, "through")
            }
            _ => text(l, "date") > text(scope, "date") && text(l, "date") <= text(scope, "through"),
        })
        .collect();
    let sources = evidence(store, state, &course_id)?;
    require_ids(list(&data, "contentSourceIds"), &sources, "Content sources")?;
    require_ids(list(&data, "formatSourceIds"), &sources, "Format sources")?;
    let scope_ids = if mode == "after_event" {
        list(scope, "evidenceIds").to_vec()
    } else {
        vec![]
    };
    require_ids(&scope_ids, &sources, "Event boundary")?;
    let lecture_ids: Vec<_> = selected.iter().map(|l| l["id"].clone()).collect();
    let content: Vec<_> = sources
        .iter()
        .filter(|s| {
            (!s["lectureId"].is_null() && lecture_ids.contains(&s["lectureId"]))
                || list(&data, "contentSourceIds").contains(&s["id"])
        })
        .cloned()
        .collect();
    if content
        .iter()
        .any(|s| !s["lectureId"].is_null() && !lecture_ids.contains(&s["lectureId"]))
    {
        return Err("Content source belongs to a lecture outside the exam scope".into());
    }
    if content.is_empty() {
        return Err("No evidence in the selected scope. Import the missing lectures or select relevant saved notes first.".into());
    }
    let label = match mode {
        "dates" => format!(
            "{} through {} (inclusive)",
            text(scope, "from"),
            text(scope, "through")
        ),
        "after_event" => format!(
            "After {} ({}, exclusive) through {}",
            text(scope, "event"),
            text(scope, "date"),
            text(scope, "through")
        ),
        _ => selected
            .iter()
            .map(|l| text(l, "title"))
            .collect::<Vec<_>>()
            .join("; "),
    };
    data["lectureIds"] = json!(lecture_ids);
    data["scopeLabel"] = json!(label);
    data["scopeEvidenceIds"] = json!(scope_ids);
    data["contentEvidenceIds"] = json!(content.iter().map(|s| s["id"].clone()).collect::<Vec<_>>());
    let context = sources
        .into_iter()
        .filter(|s| {
            ["contentEvidenceIds", "formatSourceIds", "scopeEvidenceIds"]
                .iter()
                .any(|key| list(&data, key).contains(&s["id"]))
        })
        .collect();
    Ok((data, context, selected))
}

pub fn preview(store: &Store, args: Value) -> Result<Value> {
    let (request, context, selected) = resolve(store, &store.read()?, args)?;
    let mut notes = vec![json!(
        "Undated Markdown excerpts require the agent to verify topical coverage before selecting them."
    )];
    if list(&request, "formatSourceIds").is_empty() {
        notes.push(json!(
            "No format evidence selected; propose an explicitly inferred practice format."
        ));
    }
    if text(&request["scope"], "mode") == "after_event" {
        notes.push(json!("Same-day lectures are excluded. Use explicit lecture IDs if the event cutoff falls within a day."));
    }
    let filtered = |key: &str| {
        context
            .iter()
            .filter(|s| list(&request, key).contains(&s["id"]))
            .cloned()
            .collect::<Vec<_>>()
    };
    Ok(
        json!({"scope":request["scope"],"scopeLabel":request["scopeLabel"],"lectures":selected.iter().map(|l|json!({"id":l["id"],"title":l["title"],"date":l["date"]})).collect::<Vec<_>>(),"contentEvidenceCount":list(&request,"contentEvidenceIds").len(),"additionalContent":filtered("contentSourceIds"),"assignmentEvidence":context.iter().filter(|s|list(&request,"contentEvidenceIds").contains(&s["id"]) && text(s,"kind")=="assignment").collect::<Vec<_>>(),"formatEvidence":filtered("formatSourceIds"),"boundaryEvidence":filtered("scopeEvidenceIds"),"questionCount":request["questionCount"],"questionTypes":request["questionTypes"],"notes":notes}),
    )
}

pub fn queue(store: &Store, args: Value) -> Result<Value> {
    store.mutate(|state| {
        let (request,context,_)=resolve(store,state,args)?;
        let job=json!({"id":id(),"courseId":request["courseId"],"kind":"exam","prompt":request["prompt"],"examRequest":request,"context":context,"status":"queued","createdAt":now()});
        let markdown=format!("# Practice exam request\n\n{}\n\nCoverage: {}\n\n{EXAM_POLICY}\n\nResolved request:\n\n{}\n",text(&request,"prompt"),text(&request,"scopeLabel"),serde_json::to_string_pretty(&request).map_err(|e|e.to_string())?);
        store.write(&format!("courses/{}/agent/jobs/{}.md",text(&job,"courseId"),text(&job,"id")),markdown.as_bytes())?;
        items_mut(state,"jobs").push(job.clone());Ok(job)
    })
}
pub fn brief(job: &Value) -> Value {
    let mut request = job["examRequest"].clone();
    if request.is_object() {
        request["contentEvidenceCount"] = json!(list(&request, "contentEvidenceIds").len());
        remove(&mut request, "contentEvidenceIds");
    }
    request
}
pub fn within(job: &Value, sources: Vec<Value>) -> Result<Vec<Value>> {
    if text(job, "kind") != "exam" {
        return Ok(sources);
    }
    let request = &job["examRequest"];
    if request.is_null() {
        return Err("Exam has no resolved scope".into());
    }
    Ok(sources
        .into_iter()
        .filter(|s| {
            ["contentEvidenceIds", "formatSourceIds", "scopeEvidenceIds"]
                .iter()
                .any(|key| list(request, key).contains(&s["id"]))
        })
        .collect())
}

pub fn persist(store: &Store, job: &mut Value, output: Value) -> Result<()> {
    let mut exam =
        crate::mcp::validate_schema(&crate::mcp::catalog()["outputSchemas"]["exam"], output)?;
    let request = &job["examRequest"];
    if request.is_null() {
        return Err("Exam has no resolved scope".into());
    }
    if list(&exam, "questions").len() != request["questionCount"].as_u64().unwrap_or(0) as usize {
        return Err("Exam question count does not match the request".into());
    }
    let context = list(job, "context");
    let content: Vec<_> = context
        .iter()
        .filter(|s| list(request, "contentEvidenceIds").contains(&s["id"]))
        .cloned()
        .collect();
    let format_sources: Vec<_> = context
        .iter()
        .filter(|s| list(request, "formatSourceIds").contains(&s["id"]))
        .cloned()
        .collect();
    require_ids(
        list(&exam["format"], "citations"),
        &format_sources,
        "Exam format",
    )?;
    if list(&exam["format"], "citations").is_empty() && exam["format"]["inferred"] != true {
        return Err("An unsupported exam format must be labeled inferred".into());
    }
    for kind in list(request, "questionTypes") {
        if !list(&exam, "questions").iter().any(|q| q["type"] == *kind) {
            return Err(format!(
                "Missing requested question type: {}",
                kind.as_str().unwrap_or("")
            ));
        }
    }
    for q in list(&exam, "questions") {
        if !list(request, "questionTypes").is_empty()
            && !list(request, "questionTypes").contains(&q["type"])
        {
            return Err("Unexpected question type".into());
        }
        require_ids(list(q, "citations"), &content, "Question citations (scope)")?;
        if list(q, "rubric")
            .iter()
            .map(|r| r["points"].as_u64().unwrap_or(0))
            .sum::<u64>()
            != q["points"].as_u64().unwrap_or(0)
        {
            return Err("Rubric points must sum to question points".into());
        }
        if ["multiple_choice", "multiple_select", "true_false"].contains(&text(q, "type")) {
            let choices = list(q, "choices");
            let keys: HashSet<_> = choices.iter().map(|c| text(c, "id")).collect();
            let correct = strings(q, "correctChoiceIds");
            if choices.len() < 2
                || keys.len() != choices.len()
                || correct.iter().collect::<HashSet<_>>().len() != correct.len()
                || correct.iter().any(|id| !keys.contains(id.as_str()))
            {
                return Err("Invalid choice IDs or options".into());
            }
            if if text(q, "type") == "multiple_select" {
                correct.len() < 2
            } else {
                correct.len() != 1
            } {
                return Err("Incorrect number of correct choices".into());
            }
            if text(q, "type") == "true_false"
                && (choices.len() != 2
                    || ["true", "false"].iter().any(|value| {
                        !choices
                            .iter()
                            .any(|c| text(c, "text").trim().to_lowercase() == *value)
                    }))
            {
                return Err("True/false requires True and False choices".into());
            }
        } else if !list(q, "choices").is_empty() || !list(q, "correctChoiceIds").is_empty() {
            return Err("Written questions cannot have choice answers".into());
        }
    }
    let tested: HashSet<String> = list(&exam, "questions")
        .iter()
        .flat_map(|q| list(q, "citations"))
        .filter_map(|id| content.iter().find(|s| s["id"] == *id))
        .map(|s| text(s, "lectureId").to_owned())
        .collect();
    for lecture_id in strings(request, "lectureIds") {
        if !tested.contains(&lecture_id) {
            items_mut(&mut exam,"gaps").push(json!(format!("No question directly cites selected lecture {lecture_id}; review coverage before treating this as a complete practice set.")));
        }
    }
    let references = |ids: &[Value]| {
        ids.iter()
            .filter_map(|id| context.iter().find(|c| c["id"] == *id))
            .map(|s| {
                format!(
                    "- {} [{}]{}",
                    text(s, "title").replace(['\r', '\n'], " "),
                    text(s, "id"),
                    s["url"]
                        .as_str()
                        .filter(|u| !u.is_empty())
                        .map(|u| format!(" — {u}"))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let total: u64 = list(&exam, "questions")
        .iter()
        .map(|q| q["points"].as_u64().unwrap_or(0))
        .sum();
    let questions = list(&exam, "questions")
        .iter()
        .enumerate()
        .map(|(i, q)| {
            format!(
                "## {}. {} ({} points · {})\n\n{}\n\n{}",
                i + 1,
                text(q, "topic"),
                q["points"],
                text(q, "type"),
                text(q, "prompt"),
                list(q, "choices")
                    .iter()
                    .map(|c| format!("- **{}.** {}", text(c, "id"), text(c, "text")))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let paper = format!(
        "# {}\n\nPractice exam · {total} points{}\n\nCoverage: {}\n\n{}\n\n{questions}",
        text(&exam, "title"),
        request["durationMinutes"]
            .as_u64()
            .map(|m| format!(" · {m} minutes"))
            .unwrap_or_default(),
        text(request, "scopeLabel"),
        text(&exam, "instructions")
    );
    let answers = list(&exam, "questions")
        .iter()
        .enumerate()
        .map(|(i, q)| {
            format!(
                "## {}. {}\n\n{}{}\n\n{}\n\nRubric:\n{}\n\nEvidence:\n{}",
                i + 1,
                text(q, "topic"),
                if list(q, "correctChoiceIds").is_empty() {
                    String::new()
                } else {
                    format!(
                        "Correct choices: {}\n\n",
                        strings(q, "correctChoiceIds").join(", ")
                    )
                },
                text(q, "answer"),
                text(q, "explanation"),
                list(q, "rubric")
                    .iter()
                    .map(|r| format!("- {} points: {}", r["points"], text(r, "criterion")))
                    .collect::<Vec<_>>()
                    .join("\n"),
                references(list(q, "citations"))
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let key = format!(
        "# {} — Answer key\n\n{answers}\n\n## Format and coverage\n\n{}{}\n\n{}\n\n{}\n\n{}\n",
        text(&exam, "title"),
        if exam["format"]["inferred"] == true {
            "Inferred/adapted format. "
        } else {
            "Source-supported format. "
        },
        text(&exam["format"], "rationale"),
        references(list(&exam["format"], "citations")),
        references(list(request, "scopeEvidenceIds")),
        strings(&exam, "gaps")
            .iter()
            .map(|g| format!("- {g}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let base = format!(
        "courses/{}/exams/{}",
        text(job, "courseId"),
        text(job, "id")
    );
    store.write(&format!("{base}/exam.md"), paper.as_bytes())?;
    store.write(&format!("{base}/answer-key.md"), key.as_bytes())?;
    let citations = unique(
        list(&exam, "questions")
            .iter()
            .flat_map(|q| list(q, "citations").iter().cloned())
            .chain(list(&exam["format"], "citations").iter().cloned())
            .chain(list(request, "scopeEvidenceIds").iter().cloned()),
    );
    let result = json!({"markdown":format!("{}: {} questions, {total} points. Coverage: {}.",text(&exam,"title"),list(&exam,"questions").len(),text(request,"scopeLabel")),"citations":citations,"gaps":exam["gaps"]});
    job["exam"] = exam;
    job["result"] = result;
    Ok(())
}
