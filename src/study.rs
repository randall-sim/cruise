//! Study jobs, guide authoring, practice exams, and source-grounded reconstructions.
use crate::core::{all_evidence, evidence, rank_evidence};
use crate::store::{Result, Store, arg, id, now, require_course, valid_id};
use serde_json::{Value, json};
use std::{collections::HashSet, fs};

#[path = "study_exams.rs"]
mod exams;
#[path = "study_policy.rs"]
mod policy;
#[path = "study_visuals.rs"]
mod visuals;
pub use policy::{EXAM_POLICY, POLICY};

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn strings(value: &Value, key: &str) -> Vec<String> {
    list(value, key)
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}
fn items_mut<'a>(value: &'a mut Value, key: &str) -> &'a mut Vec<Value> {
    if !value[key].is_array() {
        value[key] = json!([]);
    }
    value[key].as_array_mut().expect("array initialized")
}
fn index(state: &Value, collection: &str, identity: &str) -> Result<usize> {
    list(state, collection)
        .iter()
        .position(|v| text(v, "id") == identity)
        .ok_or_else(|| format!("{} not found", collection.trim_end_matches('s')))
}
fn lecture(state: &Value, course_id: &str, lecture_id: &str) -> Result<Value> {
    require_course(state, course_id)?;
    list(state, "lectures")
        .iter()
        .find(|v| text(v, "courseId") == course_id && text(v, "id") == lecture_id)
        .cloned()
        .ok_or_else(|| "Lecture not found in this course".into())
}
fn remove(value: &mut Value, key: &str) {
    if let Some(o) = value.as_object_mut() {
        o.remove(key);
    }
}
fn timestamp(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}
fn unique(values: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|v| seen.insert(v.to_string()))
        .collect()
}
fn write_json(store: &Store, path: &str, value: &Value) -> Result<()> {
    store.write(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
}
pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "job.create"
            | "job.retry"
            | "queue_study_job"
            | "get_job"
            | "complete_job"
            | "extend_job_evidence"
            | "list_pending_jobs"
            | "retry_job"
            | "fail_job"
            | "delete_lecture_guide"
            | "prepare_lecture_agent"
            | "get_lecture_status"
            | "get_task_workflow"
            | "prepare_exam"
            | "preview_exam_scope"
            | "queue_exam"
            | "save_capture_artifact"
            | "review_capture_readability"
            | "get_capture_artifacts"
    )
}
pub fn call(store: &Store, name: &str, args: Value) -> Result<Value> {
    match name {
        "job.create" => create_job(store, args),
        "job.retry" => {
            call(store, "retry_job", json!({"jobId":args["id"]}))?;
            Ok(Value::Null)
        }
        "queue_study_job" => {
            let job = create_job(store, args)?;
            Ok(
                json!({"id":job["id"],"status":job["status"],"evidenceCount":list(&job,"context").len()}),
            )
        }
        "complete_job" => {
            let job = complete_job(store, arg(&args, "jobId")?, args["output"].clone())?;
            Ok(json!({"id":job["id"],"status":job["status"]}))
        }
        "get_job" => get_job(store, &args),
        "extend_job_evidence" => extend_evidence(store, &args),
        "list_pending_jobs" => Ok(Value::Array(
            list(&store.read()?, "jobs")
                .iter()
                .filter(|j| text(j, "status") == "queued")
                .map(|j| {
                    let mut result = j.clone();
                    for k in ["context", "exam", "examRequest"] {
                        remove(&mut result, k);
                    }
                    result["evidenceCount"] = json!(list(j, "context").len());
                    result
                })
                .collect(),
        )),
        "retry_job" | "fail_job" => store.mutate(|state| {
            let i = index(state, "jobs", arg(&args, "jobId")?)?;
            let job = &mut state["jobs"][i];
            if name == "retry_job" {
                if text(job, "status") != "failed" {
                    return Err("Only failed jobs can be retried".into());
                }
                job["status"] = json!("queued");
                remove(job, "error");
            } else {
                if text(job, "status") != "queued" {
                    return Err("Job is not queued".into());
                }
                job["status"] = json!("failed");
                job["error"] = json!(arg(&args, "reason")?);
            }
            Ok(json!({"id":job["id"],"status":job["status"]}))
        }),
        "delete_lecture_guide" => delete_guide(store, &args),
        "prepare_lecture_agent" => prepare_lecture_agent(store, &args),
        "get_lecture_status" => {
            lecture_status(store, arg(&args, "courseId")?, arg(&args, "lectureId")?)
        }
        "get_task_workflow" => task_workflow(store, &args),
        "prepare_exam" => exams::prepare(store, &args),
        "preview_exam_scope" => exams::preview(store, args),
        "queue_exam" => {
            let job = exams::queue(store, args)?;
            Ok(
                json!({"id":job["id"],"status":job["status"],"evidenceCount":list(&job,"context").len(),"examRequest":exams::brief(&job)}),
            )
        }
        "save_capture_artifact" => visuals::save(store, args),
        "review_capture_readability" => visuals::review(store, args),
        "get_capture_artifacts" => {
            let state = store.read()?;
            let l = list(&state, "lectures")
                .iter()
                .find(|l| l["id"] == args["lectureId"])
                .ok_or("Capture not found in this lecture")?;
            let c = list(l, "captures")
                .iter()
                .find(|c| c["id"] == args["captureId"])
                .ok_or("Capture not found in this lecture")?;
            let mut result = json!({"lectureId":args["lectureId"],"captureId":args["captureId"],"artifacts":list(c,"artifacts")});
            if let Some(review) = c.get("readability") {
                result["review"] = review.clone();
            }
            Ok(result)
        }
        _ => Err(format!("Unknown study tool: {name}")),
    }
}

pub fn create_job(store: &Store, args: Value) -> Result<Value> {
    let args = crate::mcp::validate_input("queue_study_job", args)?;
    let course_id = arg(&args, "courseId")?;
    let prompt = arg(&args, "prompt")?.trim();
    if prompt.is_empty() {
        return Err("Prompt cannot be empty".into());
    }
    let kind = arg(&args, "kind")?;
    let snapshot = store.read()?;
    require_course(&snapshot, course_id)?;
    let selected = if let Some(lecture_id) = args["lectureId"].as_str() {
        Some(lecture(&snapshot, course_id, lecture_id)?)
    } else {
        None
    };
    if kind == "lecture" && selected.is_none() {
        return Err("A lecture is required".into());
    }
    let all = evidence(store, &snapshot, course_id)?;
    let context = if kind == "lecture" {
        let l = selected.as_ref().unwrap();
        let mut own: Vec<_> = all
            .iter()
            .filter(|s| s["lectureId"] == l["id"])
            .cloned()
            .collect();
        let query = format!(
            "{} {}",
            text(l, "title"),
            list(l, "cues")
                .iter()
                .take(8)
                .map(|c| text(c, "text"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        own.extend(rank_evidence(
            all.into_iter()
                .filter(|s| s["lectureId"] != l["id"])
                .collect(),
            &query,
            12,
        ));
        own
    } else {
        let mut context = rank_evidence(all.clone(), prompt, 30);
        if kind == "assessment"
            && let Some(l) = selected.as_ref()
        {
            for s in all
                .into_iter()
                .filter(|s| s["lectureId"] == l["id"] && text(s, "kind") == "transcript")
            {
                if !context.iter().any(|c| c["id"] == s["id"]) {
                    context.push(s);
                }
            }
        }
        context
    };
    store.mutate(|state| {
        require_course(state, course_id)?;
        if let Some(old) = list(state, "jobs").iter().find(|j| {
            text(j, "status") == "queued"
                && text(j, "kind") == kind
                && text(j, "courseId") == course_id
                && j["lectureId"] == args["lectureId"]
                && text(j, "prompt") == prompt
        }) {
            return Ok(old.clone());
        }
        let mut job = args.clone();
        job["id"] = json!(id());
        job["prompt"] = json!(prompt);
        job["context"] = json!(context);
        job["status"] = json!("queued");
        job["createdAt"] = json!(now());
        let markdown = format!(
            "# {kind} job\n\n{POLICY}\n\n## Request\n\n{prompt}\n\n## Evidence\n\n{}",
            context
                .iter()
                .map(|c| format!(
                    "### {} — {}{}\n{}",
                    text(c, "id"),
                    text(c, "title"),
                    c["seconds"]
                        .as_f64()
                        .map(|s| format!(" ({})", timestamp(s)))
                        .unwrap_or_default(),
                    text(c, "text")
                ))
                .collect::<Vec<_>>()
                .join("\n\n")
        );
        store.write(
            &format!("courses/{course_id}/agent/jobs/{}.md", text(&job, "id")),
            markdown.as_bytes(),
        )?;
        items_mut(state, "jobs").push(job.clone());
        Ok(job)
    })
}

fn validate_citations(ids: &[Value], context: &[Value], label: &str) -> Result<()> {
    if ids
        .iter()
        .any(|id| !context.iter().any(|source| source["id"] == *id))
    {
        Err(format!(
            "{label} contains a citation outside this job's evidence"
        ))
    } else {
        Ok(())
    }
}

fn validate_guide(mut output: Value) -> Result<Value> {
    let mut schema = crate::mcp::catalog()["outputSchemas"]["lecture"].clone();
    // Old callers may still complete saved written-response guides; new guide contracts expose choices.
    if !list(&output, "questions")
        .iter()
        .any(|q| q.get("options").is_some())
    {
        schema["properties"]["questions"] = json!({"type":"array","maxItems":30,"items":{"type":"object","required":["question","answer","citations"],"additionalProperties":false,"properties":{"question":{"type":"string","maxLength":3000},"answer":{"type":"string","maxLength":5000},"citations":{"type":"array","minItems":1,"maxItems":100,"items":{"type":"string"}}}}});
    }
    output = crate::mcp::validate_schema(&schema, output)?;
    for q in list(&output, "questions") {
        if let Some(options) = q["options"].as_array() {
            let correct = q["correctOption"]
                .as_u64()
                .ok_or("Correct choice must exist in options")? as usize;
            if correct >= options.len() {
                return Err("Correct choice must exist in options".into());
            }
            let labels: HashSet<_> = options
                .iter()
                .map(|o| text(o, "text").trim().to_lowercase())
                .collect();
            if labels.len() != options.len() {
                return Err("Choices must be distinct".into());
            }
        }
    }
    Ok(output)
}

pub fn merge_word_bank(
    concepts: &[Value],
    entries: &[Value],
    lecture: &Value,
    evidence: &[Value],
) -> Vec<Value> {
    let mut result = concepts.to_vec();
    for entry in entries {
        let citations: Vec<_> = list(entry, "citations")
            .iter()
            .filter_map(|id| evidence.iter().find(|s| s["id"] == *id))
            .cloned()
            .collect();
        if let Some(existing) = result.iter_mut().find(|c| {
            c["courseId"] == lecture["courseId"]
                && text(c, "term").trim().to_lowercase()
                    == text(entry, "term").trim().to_lowercase()
        }) {
            if text(entry, "extensionReason").is_empty() {
                continue;
            }
            let addition = format!(
                "\n\nAddition from {} ({}; lecture {}). Why: {}\n\n{}",
                text(lecture, "title"),
                text(lecture, "date"),
                text(lecture, "id"),
                text(entry, "extensionReason"),
                text(entry, "definition")
            );
            if !text(existing, "definition").contains(&addition) {
                existing["definition"] =
                    json!(format!("{}{addition}", text(existing, "definition")));
                existing["mastered"] = json!(false);
            }
            for source in citations {
                if !list(existing, "citations")
                    .iter()
                    .any(|s| s["id"] == source["id"])
                {
                    items_mut(existing, "citations").push(source);
                }
            }
        } else {
            result.push(json!({"id":id(),"courseId":lecture["courseId"],"lectureId":lecture["id"],"term":entry["term"],"definition":entry["definition"],"citations":citations,"mastered":false}));
        }
    }
    result
}

fn heading(section: &Value) -> String {
    let pattern=regex::Regex::new(r"^(?:(?:\d{1,3}:\d{2}(?::\d{2})?\s*[–—-]\s*\d{1,3}:\d{2}(?::\d{2})?)\s*[·•:–—-]?\s*|\d+\s*[.·•:–—-]\s*)?").unwrap();
    pattern
        .replace(text(section, "title").trim(), "")
        .to_lowercase()
}
fn coverage(section: &Value) -> bool {
    text(section, "category") == "coverage"
        || [
            "coverage",
            "evidence coverage",
            "resources and coverage",
            "resources and gaps",
        ]
        .iter()
        .any(|prefix| heading(section).starts_with(prefix))
}
fn ancillary(section: &Value) -> bool {
    if !text(section, "category").is_empty() {
        return text(section, "category") != "lecture";
    }
    [
        "supplementary",
        "course logistics",
        "logistics",
        "lecture resources",
        "resources",
        "evidence coverage",
        "coverage",
        "prior-lecture connections",
        "prior lecture connections",
        "connections to earlier lectures",
        "connections to prior lectures",
        "connections to previous lectures",
    ]
    .iter()
    .any(|prefix| heading(section).starts_with(prefix))
}
fn references(ids: &[Value], context: &[Value]) -> String {
    ids.iter()
        .filter_map(|id| context.iter().find(|s| s["id"] == *id))
        .map(|c| {
            format!(
                "[{}{}](/?course={}{}{})",
                text(c, "title"),
                c["seconds"]
                    .as_f64()
                    .map(|s| format!(" · {}", timestamp(s)))
                    .unwrap_or_default(),
                text(c, "courseId"),
                c["lectureId"]
                    .as_str()
                    .map(|id| format!("&lecture={id}"))
                    .unwrap_or_default(),
                c["seconds"]
                    .as_f64()
                    .map(|s| format!("#t-{}", s.floor()))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}
fn relative_path(from_file: &str, target: &str) -> String {
    let mut from: Vec<_> = from_file.split('/').collect();
    from.pop();
    let to: Vec<_> = target.split('/').collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts = vec![".."; from.len() - common];
    parts.extend_from_slice(&to[common..]);
    parts.join("/")
}
fn sections_markdown(
    sections: &[Value],
    context: &[Value],
    notes_path: &str,
    is_ancillary: bool,
) -> String {
    sections
        .iter()
        .filter(|s| !coverage(s) && ancillary(s) == is_ancillary)
        .map(|s| {
            let images = list(s, "citations")
                .iter()
                .filter_map(|id| {
                    context
                        .iter()
                        .find(|c| c["id"] == *id && text(c, "kind") == "capture")
                })
                .map(|c| {
                    format!(
                        "![{}](<{}>)\n\n{}",
                        text(c, "title").replace(['[', ']', '\r', '\n'], " "),
                        relative_path(notes_path, text(c, "path")),
                        references(&[c["id"].clone()], context)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            format!(
                "## {}\n\n{}{}\n\n{images}\n\nSources: {}",
                text(s, "title"),
                text(s, "markdown"),
                s["fastMarkdown"]
                    .as_str()
                    .map(|v| format!("\n\n### Fast\n\n{v}"))
                    .unwrap_or_default(),
                references(list(s, "citations"), context)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
fn guide_markdown(lecture: &Value, guide: &Value, context: &[Value]) -> String {
    let questions=list(guide,"questions").iter().enumerate().map(|(i,q)| {
        let choices=list(q,"options").iter().enumerate().map(|(n,o)|format!("{}. {}",(b'A'+n as u8) as char,text(o,"text"))).collect::<Vec<_>>().join("\n\n");
        let explanations=list(q,"options").iter().enumerate().map(|(n,o)|format!("{}. {}: {}",(b'A'+n as u8) as char,if q["correctOption"].as_u64()==Some(n as u64) {"Correct"} else {"Incorrect"},text(o,"explanation"))).collect::<Vec<_>>().join("\n\n");
        format!("### {}. {}\n\n{choices}\n\n<details>\n<summary>Answer and explanation</summary>\n\n{}\n\n{explanations}\n\nSources: {}\n\n</details>",i+1,text(q,"question"),text(q,"answer"),references(list(q,"citations"),context))
    }).collect::<Vec<_>>().join("\n\n");
    format!(
        "# {}\n\n{}\n\n{}\n\n## Check your understanding\n\n{questions}\n\n{}\n\n## Course logistics\n\n{}\n",
        text(lecture, "title"),
        text(guide, "summary"),
        sections_markdown(
            list(guide, "sections"),
            context,
            text(lecture, "notesPath"),
            false
        ),
        sections_markdown(
            list(guide, "sections"),
            context,
            text(lecture, "notesPath"),
            true
        ),
        list(guide, "logistics")
            .iter()
            .map(|item| format!(
                "- {} ({})",
                text(item, "text"),
                references(list(item, "citations"), context)
            ))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

pub fn complete_job(store: &Store, job_id: &str, output: Value) -> Result<Value> {
    valid_id(job_id)?;
    store.mutate(|state| {
        let i=index(state,"jobs",job_id)?;
        let mut job=state["jobs"][i].clone();
        if text(&job,"status")!="queued" {return Err("Job is not queued".into());}
        let context=list(&job,"context").to_vec();
        match text(&job,"kind") {
            "lecture" => {
                let mut guide=validate_guide(output)?;
                for key in ["sections","logistics","concepts","questions"] {for item in list(&guide,key) {validate_citations(list(item,"citations"),&context,"Result")?;}}
                let l=lecture(state,text(&job,"courseId"),text(&job,"lectureId"))?;
                let duration=l["duration"].as_f64().unwrap_or(0.0);
                for section in list(&guide,"sections") {
                    let start=section["startSeconds"].as_f64();let end=section["endSeconds"].as_f64();
                    if start.is_some()!=end.is_some() {return Err("A section transcript range needs both startSeconds and endSeconds".into());}
                    if let (Some(start),Some(end))=(start,end)&& (end<=start || start>duration || end>duration+60.0) {return Err("Section transcript range is outside the lecture or reversed".into());}
                }
                if !list(&guide,"gaps").contains(&l["captureCoverage"]) {items_mut(&mut guide,"gaps").push(l["captureCoverage"].clone());}
                let expected=format!("courses/{}/lectures/{}/notes.md",text(&job,"courseId"),text(&l,"id"));
                if text(&l,"notesPath")!=expected {return Err("Guide file path does not match this lecture".into());}
                store.write(&expected,guide_markdown(&l,&guide,&context).as_bytes())?;
                state["concepts"]=json!(merge_word_bank(list(state,"concepts"),list(&guide,"concepts"),&l,&context));
                let n=index(state,"lectures",text(&l,"id"))?;
                state["lectures"][n]["guide"]=guide.clone();state["lectures"][n]["evidence"]=json!(context);state["lectures"][n]["status"]=json!("ready");state["lectures"][n]["reviewed"]=json!(false);
                job["result"]=json!({"markdown":guide["summary"],"citations":list(&guide,"sections").iter().flat_map(|s|list(s,"citations").iter().cloned()).collect::<Vec<_>>(),"gaps":guide["gaps"]});
            },
            "exam" => exams::persist(store,&mut job,output)?,
            _ => {
                let answer=crate::mcp::validate_schema(&crate::mcp::catalog()["outputSchemas"]["answer"],output)?;
                validate_citations(list(&answer,"citations"),&context,"Result")?;
                if list(&answer,"citations").is_empty() && list(&answer,"gaps").is_empty() {return Err("An answer needs source citations or an explicit evidence gap".into());}
                if text(&job,"kind")=="assignment" {
                    crate::files::complete_assignment_job(store,state,&job,&answer)?;
                } else {
                    let markdown=format!("# {}\n\n{}\n\nSources: {}\n\n{}",text(&job,"prompt"),text(&answer,"markdown"),strings(&answer,"citations").join(", "),strings(&answer,"gaps").iter().map(|g|format!("- {g}")).collect::<Vec<_>>().join("\n"));
                    store.write(&format!("courses/{}/agent/results/{job_id}.md",text(&job,"courseId")),markdown.as_bytes())?;
                }
                job["result"]=answer;
            }
        }
        job["status"]=json!("completed");job["completedAt"]=json!(now());
        state["jobs"][i]=job.clone();Ok(job)
    })
}

fn get_job(store: &Store, args: &Value) -> Result<Value> {
    let state = store.read()?;
    let i = index(&state, "jobs", arg(args, "jobId")?)?;
    let job = &state["jobs"][i];
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;
    let limit = args["limit"].as_u64().unwrap_or(50).clamp(1, 100) as usize;
    let context = list(job, "context");
    let page: Vec<_> = context.iter().skip(offset).take(limit).cloned().collect();
    let mut brief = job.clone();
    remove(&mut brief, "context");
    if !job["examRequest"].is_null() {
        brief["examRequest"] = exams::brief(job);
    }
    let kind = if text(job, "kind") == "lecture" {
        "lecture"
    } else if text(job, "kind") == "exam" {
        "exam"
    } else {
        "answer"
    };
    let mut result = json!({"policy":if kind=="exam" {format!("{POLICY}\n\n{EXAM_POLICY}")} else {POLICY.to_owned()},"job":brief,"course":require_course(&state,text(job,"courseId"))?,"outputSchema":crate::mcp::catalog()["outputSchemas"][kind],"evidence":page,"total":context.len(),"nextOffset":if offset.saturating_add(limit)<context.len(){json!(offset+limit)}else{Value::Null}});
    if let Some(l) = list(&state, "lectures")
        .iter()
        .find(|l| l["id"] == job["lectureId"])
    {
        result["coverage"] = l["captureCoverage"].clone();
    }
    if kind == "exam" {
        result["contentEvidenceIds"] = json!(
            page.iter()
                .filter(|s| list(&job["examRequest"], "contentEvidenceIds").contains(&s["id"]))
                .map(|s| s["id"].clone())
                .collect::<Vec<_>>()
        );
    }
    Ok(result)
}
fn extend_evidence(store: &Store, args: &Value) -> Result<Value> {
    let job_id = arg(args, "jobId")?;
    let query = arg(args, "query")?;
    let state = store.read()?;
    let snapshot = &state["jobs"][index(&state, "jobs", job_id)?];
    if text(snapshot, "status") != "queued" {
        return Err("Job is not queued".into());
    }
    let sources = exams::within(snapshot, all_evidence(store, text(snapshot, "courseId"))?)?;
    let matches = rank_evidence(sources, query, 20);
    store.mutate(|state| {
        let i = index(state, "jobs", job_id)?;
        let job = &mut state["jobs"][i];
        if text(job, "status") != "queued" {
            return Err("Job is not queued".into());
        }
        let added: Vec<_> = matches
            .into_iter()
            .filter(|s| !list(job, "context").iter().any(|c| c["id"] == s["id"]))
            .collect();
        items_mut(job, "context").extend(added.clone());
        Ok(json!({"added":added,"total":list(job,"context").len()}))
    })
}

fn delete_guide(store: &Store, args: &Value) -> Result<Value> {
    let course_id = arg(args, "courseId")?;
    let lecture_id = arg(args, "lectureId")?;
    store.mutate(|state| {
        let l=lecture(state,course_id,lecture_id)?;
        if l["guide"].is_null() {return Ok(json!({"lectureId":lecture_id,"deleted":false}));}
        let notes_path=format!("courses/{course_id}/lectures/{lecture_id}/notes.md");
        if text(&l,"notesPath")!=notes_path {return Err("Guide file path does not match this lecture".into());}
        let notes_file=store.path(&notes_path)?;
        let markdown=match fs::read_to_string(&notes_file) {Ok(s)=>json!(s),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Value::Null,Err(e)=>return Err(e.to_string())};
        let belongs=|j:&Value|text(j,"lectureId")==lecture_id && text(j,"courseId")==course_id;
        let jobs:Vec<_>=list(state,"jobs").iter().filter(|j|text(j,"kind")=="lecture" && belongs(j)).cloned().collect();
        let concepts:Vec<_>=list(state,"concepts").iter().filter(|c|belongs(c)).cloned().collect();
        let recovery=format!(".trash/guides/{course_id}/{lecture_id}/{}.json",id());
        write_json(store,&recovery,&json!({"deletedAt":now(),"courseId":course_id,"lectureId":lecture_id,"notesPath":notes_path,"markdown":markdown,"guide":l["guide"],"evidence":l["evidence"],"reviewed":l["reviewed"],"jobs":jobs,"concepts":concepts}))?;
        match fs::remove_file(notes_file) {Ok(())=>(),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>(),Err(e)=>return Err(e.to_string())}
        let i=index(state,"lectures",lecture_id)?;
        remove(&mut state["lectures"][i],"guide");remove(&mut state["lectures"][i],"evidence");state["lectures"][i]["status"]=json!("imported");state["lectures"][i]["reviewed"]=json!(false);
        items_mut(state,"concepts").retain(|c|!belongs(c));
        items_mut(state,"jobs").retain(|j|!(text(j,"kind")=="lecture" && belongs(j)));
        Ok(json!({"lectureId":lecture_id,"deleted":true,"recoveryPath":recovery,"removedGuideJobs":jobs.len()}))
    })
}

fn prepare_lecture_agent(store: &Store, args: &Value) -> Result<Value> {
    let state = store.read()?;
    let course_id = arg(args, "courseId")?;
    require_course(&state, course_id)?;
    let found = list(&state, "lectures").iter().find(|l| {
        text(l, "courseId") == course_id
            && if let Some(id) = args["lectureId"].as_str() {
                text(l, "id") == id
            } else {
                !text(args, "sourceUrl").is_empty() && l["sourceUrl"] == args["sourceUrl"]
            }
    });
    if args["lectureId"].is_string() && found.is_none() {
        return Err("Lecture not found in this course".into());
    }
    if found.is_none()
        && ["title", "date", "sourceUrl"]
            .iter()
            .any(|k| text(args, k).is_empty())
    {
        return Err("Provide lectureId, or title, date and sourceUrl for one recording".into());
    }
    let brief=found.map(|l|json!({"courseId":l["courseId"],"lectureId":l["id"],"title":l["title"],"date":l["date"],"sourceUrl":l["sourceUrl"],"status":l["status"],"rebuild":args["rebuild"].as_bool().unwrap_or(false)})).unwrap_or_else(||args.clone());
    Ok(
        json!({"status":"prepared_not_spawned","nativeTool":"collaboration.spawn_agent","customAgent":"lecture_summary","spawnArguments":{"task_name":format!("lecture_{}",found.map(|l|text(l,"id").to_owned()).unwrap_or_else(id).replace('-',"_")),"model":"gpt-6-astra","reasoning_effort":"medium","fork_turns":"none","message":format!("{}{}",policy::LECTURE_AGENT,brief)},"nextStep":"Call the native spawn tool with spawnArguments. Wait before dispatching another browser worker. Preparation alone does not start an agent."}),
    )
}
pub fn lecture_status(store: &Store, course_id: &str, lecture_id: &str) -> Result<Value> {
    let state = store.read()?;
    let l = lecture(&state, course_id, lecture_id)?;
    let capture_ids: HashSet<_> = list(&l, "captures")
        .iter()
        .map(|c| format!("{lecture_id}:c:{}", text(c, "id")))
        .collect();
    let sections = list(&l["guide"], "sections");
    let visual = sections
        .iter()
        .filter(|s| {
            strings(s, "citations")
                .iter()
                .any(|id| capture_ids.contains(id))
        })
        .count();
    let linked: HashSet<_> = sections
        .iter()
        .flat_map(|s| strings(s, "citations"))
        .filter(|id| capture_ids.contains(id))
        .collect();
    Ok(
        json!({"courseId":course_id,"lectureId":lecture_id,"title":l["title"],"status":l["status"],"notesPath":l["notesPath"],"hasGuide":!l["guide"].is_null(),"captureCount":list(&l,"captures").len(),"sectionCount":sections.len(),"visualPageCount":visual,"textPageCount":sections.len()-visual,"linkedCaptureCount":linked.len(),"coverage":l["captureCoverage"],"gaps":list(&l["guide"],"gaps"),"jobs":list(&state,"jobs").iter().filter(|j|text(j,"courseId")==course_id && text(j,"lectureId")==lecture_id).map(|j|{let mut v=json!({"id":j["id"],"status":j["status"]});if j["error"].is_string(){v["error"]=j["error"].clone();}v}).collect::<Vec<_>>()}),
    )
}
fn task_workflow(store: &Store, args: &Value) -> Result<Value> {
    let course = require_course(&store.read()?, arg(args, "courseId")?)?;
    Ok(
        json!({"status":"browser_action_required","courseId":course["id"],"course":{"code":course["code"],"name":course["name"],"term":course["term"]},"request":args["request"],"intent":args["intent"].as_str().unwrap_or("check"),"sources":{"canvas":if text(&course,"canvasUrl").is_empty(){Value::Null}else{course["canvasUrl"].clone()},"website":if text(&course,"websiteUrl").is_empty(){Value::Null}else{course["websiteUrl"].clone()}},"instructions":policy::TASK_WORKFLOW,"missingLinks":"If a source URL is not configured, use the matching open official course tab or browser navigation. Ask for the location only if it cannot be identified. Save confirmed URLs with update_course. Never infer task status from missing access."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(Store);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.root);
        }
    }
    fn fixture() -> Fixture {
        let store = Store {
            root: std::env::temp_dir().join(format!("cruise-study-test-{}", id())),
        };
        store.mutate(|state| {
            state["courses"]=json!([{"id":"course","code":"CS101","name":"Algorithms","term":"Fall 2026","canvasUrl":"https://example.edu/course","websiteUrl":""},{"id":"other","code":"OTHER","name":"Other","term":""}]);
            state["lectures"]=json!(["2026-09-30","2026-10-01","2026-10-15","2026-10-16","2026-11-30","2026-12-01"].iter().enumerate().map(|(i,date)|json!({"id":format!("l{i}"),"courseId":"course","title":format!("Lecture {}",i+1),"date":date,"sourceUrl":"https://example.edu/video","duration":20,"status":"imported","reviewed":false,"captureCoverage":"Transcript only; no visual inspection.","notesPath":format!("courses/course/lectures/l{i}/notes.md"),"transcriptPath":format!("courses/course/lectures/l{i}/transcript.vtt"),"cues":[{"start":0,"end":20,"text":"Binary search halves a sorted interval; ordering tells us which half cannot contain the target."}],"captures":[]})).collect::<Vec<_>>());
            state["futureField"]=json!({"preserved":true});Ok(())
        }).unwrap();
        Fixture(store)
    }
    fn guide() -> Value {
        json!({"summary":"Ordering supports efficient binary search on a sorted interval.","sections":[{"title":"Search","category":"lecture","markdown":"Compare the target with the middle value, then use ordering to discard half.","fastMarkdown":"- Compare the middle.\n- Discard the impossible half.\n- Ordering is essential.","citations":["l1:t0"],"startSeconds":0,"endSeconds":20}],"logistics":[],"concepts":[{"term":"Ordering","definition":"Values are in a sorted sequence.","citations":["l1:t0"]}],"questions":[],"gaps":[]})
    }
    fn exam_output(citation: &str) -> Value {
        json!({"title":"Search practice","instructions":"Attempt both questions before revealing answers.","format":{"inferred":true,"rationale":"Inferred practice mix.","citations":[]},"questions":[{"type":"multiple_choice","topic":"Search condition","prompt":"Which property permits halving?","points":2,"choices":[{"id":"A","text":"The interval is sorted"},{"id":"B","text":"Values are random"}],"correctChoiceIds":["A"],"answer":"A sorted interval.","explanation":"Ordering rules out the impossible half.","rubric":[{"criterion":"Select the sorted interval","points":2}],"citations":[citation]},{"type":"short_answer","topic":"Halving","prompt":"Why can half be discarded?","points":3,"choices":[],"correctChoiceIds":[],"answer":"Compare the middle.","explanation":"Ordering rules out the impossible half.","rubric":[{"criterion":"Middle comparison","points":1},{"criterion":"Explain discarded half","points":2}],"citations":[citation]}],"gaps":[]})
    }

    #[test]
    fn job_evidence_validation_pagination_failure_retry_and_persistence() {
        let f = fixture();
        let store = &f.0;
        let input = json!({"courseId":"course","kind":"question","prompt":"Why does binary search need ordering?"});
        let job = create_job(store, input.clone()).unwrap();
        assert_eq!(create_job(store, input).unwrap()["id"], job["id"]);
        let job_id = text(&job, "id");
        let first = call(store, "get_job", json!({"jobId":job_id,"limit":1})).unwrap();
        assert_eq!(list(&first, "evidence").len(), 1);
        assert_eq!(first["nextOffset"], 1);
        assert!(first["job"].get("context").is_none());
        assert!(
            complete_job(
                store,
                job_id,
                json!({"markdown":"Unsupported","citations":["foreign:t0"],"gaps":[]})
            )
            .is_err()
        );
        assert!(
            complete_job(
                store,
                job_id,
                json!({"markdown":"Unsupported","citations":[],"gaps":[]})
            )
            .is_err()
        );
        call(
            store,
            "fail_job",
            json!({"jobId":job_id,"reason":"Missing context"}),
        )
        .unwrap();
        assert!(
            complete_job(
                store,
                job_id,
                json!({"markdown":"Answer","citations":["l1:t0"],"gaps":[]})
            )
            .is_err()
        );
        call(store, "retry_job", json!({"jobId":job_id})).unwrap();
        let done = complete_job(
            store,
            job_id,
            json!({"markdown":"Ordering permits excluding half.","citations":["l1:t0"],"gaps":[]}),
        )
        .unwrap();
        assert_eq!(done["status"], "completed");
        assert!(
            store
                .path(&format!("courses/course/agent/results/{job_id}.md"))
                .unwrap()
                .exists()
        );
        assert_eq!(store.read().unwrap()["futureField"]["preserved"], true);
    }

    #[test]
    fn guides_validate_ranges_choices_and_citations_then_delete_with_recovery() {
        let f = fixture();
        let store = &f.0;
        let job = create_job(
            store,
            json!({"courseId":"course","kind":"lecture","lectureId":"l1","prompt":"Teach search"}),
        )
        .unwrap();
        let job_id = text(&job, "id");
        let mut bad = guide();
        bad["sections"][0]["endSeconds"] = json!(100);
        assert!(
            complete_job(store, job_id, bad)
                .unwrap_err()
                .contains("range")
        );
        let mut bad = guide();
        bad["concepts"][0]["citations"] = json!(["forged"]);
        assert!(
            complete_job(store, job_id, bad)
                .unwrap_err()
                .contains("citation")
        );
        complete_job(store, job_id, guide()).unwrap();
        let status = lecture_status(store, "course", "l1").unwrap();
        assert_eq!(status["hasGuide"], true);
        assert_eq!(status["sectionCount"], 1);
        assert_eq!(status["gaps"][0], "Transcript only; no visual inspection.");
        let pending=create_job(store,json!({"courseId":"course","kind":"lecture","lectureId":"l1","prompt":"Rebuild search"})).unwrap();
        let recovery = call(
            store,
            "delete_lecture_guide",
            json!({"courseId":"course","lectureId":"l1"}),
        )
        .unwrap();
        assert_eq!(recovery["removedGuideJobs"], 2);
        let backup: Value = serde_json::from_slice(
            &fs::read(store.path(text(&recovery, "recoveryPath")).unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(backup["guide"]["summary"], guide()["summary"]);
        assert!(backup["markdown"].as_str().unwrap().contains("### Fast"));
        assert_eq!(
            lecture_status(store, "course", "l1").unwrap()["status"],
            "imported"
        );
        assert!(complete_job(store, text(&pending, "id"), guide()).is_err());
        assert!(list(&store.read().unwrap(), "concepts").is_empty());
    }

    #[test]
    fn multiple_choice_guides_reject_bad_indices_and_duplicate_options() {
        let q = json!({"question":"What is required?","answer":"Sorted order.","options":[{"text":"Sorted","explanation":"Correct, it permits exclusion."},{"text":"Random","explanation":"Incorrect, no ordering guarantee."}],"correctOption":0,"citations":["l1:t0"]});
        let mut output = guide();
        output["questions"] = json!(vec![q; 8]);
        assert!(validate_guide(output.clone()).is_ok());
        output["questions"][0]["correctOption"] = json!(2);
        assert!(
            validate_guide(output.clone())
                .unwrap_err()
                .contains("exist")
        );
        output["questions"][0]["correctOption"] = json!(0);
        output["questions"][0]["options"][1]["text"] = json!("SORTED");
        assert!(validate_guide(output).unwrap_err().contains("distinct"));
    }

    #[test]
    fn word_bank_preserves_original_and_idempotently_appends_cited_extensions() {
        let l = json!({"id":"l2","courseId":"course","title":"Inference","date":"2026-10-01"});
        let evidence = vec![json!({"id":"l2:t0","courseId":"course","kind":"transcript"})];
        let old = json!({"id":"original","courseId":"course","lectureId":"l1","term":"Hypothesis","definition":"A testable explanation.","citations":[],"mastered":true});
        let entry = json!({"term":" HYPOTHESIS ","definition":"An additional supported property.","citations":["l2:t0"]});
        assert_eq!(
            merge_word_bank(
                std::slice::from_ref(&old),
                std::slice::from_ref(&entry),
                &l,
                &evidence
            ),
            vec![old.clone()]
        );
        let mut addition = entry;
        addition["extensionReason"] = json!("The current lecture expands the meaning.");
        let result = merge_word_bank(
            std::slice::from_ref(&old),
            &[addition.clone()],
            &l,
            &evidence,
        );
        assert_eq!(result[0]["id"], "original");
        assert!(text(&result[0], "definition").starts_with(text(&old, "definition")));
        assert_eq!(result[0]["mastered"], false);
        assert_eq!(result[0]["citations"], json!(evidence));
        assert_eq!(merge_word_bank(&result, &[addition], &l, &evidence), result);
    }

    #[test]
    fn exams_resolve_inclusive_dates_exclusive_event_cutoffs_and_frozen_scope() {
        let f = fixture();
        let store = &f.0;
        let input = json!({"courseId":"course","prompt":"October through November","scope":{"mode":"dates","from":"2026-10-01","through":"2026-11-30"}});
        let before = fs::read(store.path("state.json").unwrap()).unwrap();
        let preview = exams::preview(store, input.clone()).unwrap();
        assert_eq!(
            strings(
                &json!({"ids":list(&preview,"lectures").iter().map(|l|l["id"].clone()).collect::<Vec<_>>()}),
                "ids"
            ),
            vec!["l1", "l2", "l3", "l4"]
        );
        assert_eq!(fs::read(store.path("state.json").unwrap()).unwrap(), before);
        let mut invalid = input.clone();
        invalid["scope"]["from"] = json!("2026-02-30");
        assert!(exams::preview(store, invalid).is_err());
        let mut invalid = input.clone();
        invalid["contentSourceIds"] = json!(["l0:t0"]);
        assert!(
            exams::preview(store, invalid)
                .unwrap_err()
                .contains("outside")
        );
        let after=exams::preview(store,json!({"courseId":"course","prompt":"After midterm","scope":{"mode":"after_event","event":"Midterm","date":"2026-10-15","through":"2026-11-30","evidenceIds":["l2:t0"]}})).unwrap();
        assert_eq!(list(&after, "lectures").len(), 2);
        assert_eq!(after["lectures"][0]["id"], "l3");
        let job=exams::queue(store,json!({"courseId":"course","prompt":"Practice search","questionCount":2,"scope":{"mode":"lectures","lectureIds":["l3"]},"formatSourceIds":["l0:t0"]})).unwrap();
        let filtered = exams::within(&job, all_evidence(store, "course").unwrap()).unwrap();
        assert_eq!(filtered.len(), 2);
        assert!(!filtered.iter().any(|s| s["lectureId"] == "l1"));
        assert!(
            complete_job(store, text(&job, "id"), exam_output("l0:t0"))
                .unwrap_err()
                .contains("scope")
        );
    }

    #[test]
    fn exam_completion_validates_type_mix_choice_ids_points_and_answer_separation() {
        let f = fixture();
        let store = &f.0;
        let job=exams::queue(store,json!({"courseId":"course","prompt":"Mixed search","questionCount":2,"questionTypes":["multiple_choice","short_answer"],"scope":{"mode":"lectures","lectureIds":["l1","l2"]}})).unwrap();
        let job_id = text(&job, "id");
        let valid = exam_output("l1:t0");
        let mut bad = valid.clone();
        bad["questions"][0]["correctChoiceIds"] = json!(["C"]);
        assert!(
            complete_job(store, job_id, bad)
                .unwrap_err()
                .contains("choice IDs")
        );
        let mut bad = valid.clone();
        bad["questions"][0]["rubric"][0]["points"] = json!(1);
        assert!(
            complete_job(store, job_id, bad)
                .unwrap_err()
                .contains("Rubric")
        );
        let mut bad = valid.clone();
        bad["format"]["inferred"] = json!(false);
        assert!(
            complete_job(store, job_id, bad)
                .unwrap_err()
                .contains("inferred")
        );
        let mut bad = valid.clone();
        bad["questions"][1]["type"] = json!("essay");
        assert!(
            complete_job(store, job_id, bad)
                .unwrap_err()
                .contains("Missing requested")
        );
        let done = complete_job(store, job_id, valid).unwrap();
        assert_eq!(done["status"], "completed");
        assert!(
            strings(&done["exam"], "gaps")
                .iter()
                .any(|g| g.contains("l2"))
        );
        let paper = fs::read_to_string(
            store
                .path(&format!("courses/course/exams/{job_id}/exam.md"))
                .unwrap(),
        )
        .unwrap();
        let key = fs::read_to_string(
            store
                .path(&format!("courses/course/exams/{job_id}/answer-key.md"))
                .unwrap(),
        )
        .unwrap();
        assert!(!paper.contains("Ordering rules out"));
        assert!(key.contains("Ordering rules out"));
        assert!(key.contains("l1:t0"));
    }

    #[test]
    fn all_exam_question_types_and_scoped_evidence_extension_work() {
        let f = fixture();
        let store = &f.0;
        let kinds = [
            "multiple_choice",
            "multiple_select",
            "true_false",
            "short_answer",
            "essay",
            "calculation",
            "proof",
            "code",
            "diagram",
        ];
        let job=exams::queue(store,json!({"courseId":"course","prompt":"All types","questionCount":9,"questionTypes":kinds,"scope":{"mode":"lectures","lectureIds":["l1"]}})).unwrap();
        let extension = call(
            store,
            "extend_job_evidence",
            json!({"jobId":job["id"],"query":"binary search"}),
        )
        .unwrap();
        assert_eq!(extension["total"], 1);
        assert!(list(&extension, "added").is_empty());
        let mut output = exam_output("l1:t0");
        let questions: Vec<_> = kinds.iter().map(|kind| {
            let mut question = output["questions"][if *kind == "multiple_choice" { 0 } else { 1 }].clone();
            question["type"] = json!(kind);
            if *kind == "multiple_select" {
                question["choices"] = json!([{"id":"A","text":"One"},{"id":"B","text":"Two"},{"id":"C","text":"Neither"}]);
                question["correctChoiceIds"] = json!(["A", "B"]);
            }
            if *kind == "true_false" {
                question["choices"] = json!([{"id":"T","text":"True"},{"id":"F","text":"False"}]);
                question["correctChoiceIds"] = json!(["T"]);
            }
            question
        }).collect();
        output["questions"] = json!(questions);
        let done = complete_job(store, text(&job, "id"), output).unwrap();
        assert_eq!(list(&done["exam"], "questions").len(), 9);
        let page = call(store, "get_job", json!({"jobId":job["id"]})).unwrap();
        assert_eq!(page["contentEvidenceIds"], json!(["l1:t0"]));
        assert_eq!(page["job"]["examRequest"]["contentEvidenceCount"], 1);
        assert!(
            page["job"]["examRequest"]
                .get("contentEvidenceIds")
                .is_none()
        );
    }

    #[test]
    fn assignment_job_saves_tracked_shared_draft_and_memory() {
        let f = fixture();
        let store = &f.0;
        let job = create_job(
            store,
            json!({"courseId":"course","kind":"assignment","prompt":"Explain binary search"}),
        )
        .unwrap();
        let done=complete_job(store,text(&job,"id"),json!({"markdown":"Use ordering to exclude half of the interval.","citations":["l1:t0"],"gaps":[]})).unwrap();
        assert_eq!(done["status"], "completed");
        let path = format!(
            "courses/course/files/assignments/{}/draft.md",
            text(&job, "id")
        );
        assert!(store.path(&path).unwrap().exists());
        let files = crate::files::call(
            store,
            "list_assignment_files",
            json!({"courseId":"course","assignmentId":job["id"]}),
        )
        .unwrap();
        assert!(files.to_string().contains("draft.md"));
        let history = crate::files::call(
            store,
            "get_file_history",
            json!({"courseId":"course","assignmentId":job["id"],"path":"draft.md"}),
        )
        .unwrap();
        assert!(
            history
                .to_string()
                .contains("Completed the assignment study job.")
        );
        assert!(
            all_evidence(store, "course").unwrap().iter().any(
                |s| text(s, "kind") == "assignment" && text(s, "text").contains("Use ordering")
            )
        );
    }

    #[test]
    fn reconstruction_diagrams_are_inert_bounded_and_source_grounded() {
        let f = fixture();
        let store = &f.0;
        store.mutate(|s|{s["lectures"][1]["captures"]=json!([{"id":"capture","file":"courses/course/lectures/l1/captures/capture.png","seconds":5,"kind":"whiteboard","stream":"Board","caption":"Search illustration"}]);Ok(())}).unwrap();
        store
            .write(
                "courses/course/lectures/l1/captures/capture.png",
                b"original evidence",
            )
            .unwrap();
        let diagram = json!({"width":1000,"height":500,"elements":[{"type":"text","x":40,"y":100,"lines":["<script>alert(1)</script> & <foreignObject>"]}]});
        let (svg, _) = visuals::render_diagram(diagram.clone(), "<script>title</script>").unwrap();
        assert!(!svg.contains("<script>"));
        assert!(svg.contains("&lt;script&gt;"));
        assert!(visuals::render_diagram(json!({"width":1000,"height":500,"script":"alert(1)","elements":[{"type":"rect","x":0,"y":0,"width":20,"height":20}]}),"Bad").is_err());
        assert!(visuals::render_diagram(json!({"width":1000,"height":500,"elements":[{"type":"rect","x":990,"y":0,"width":20,"height":20}]}),"Bad").unwrap_err().contains("canvas"));
        let input = json!({"lectureId":"l1","captureId":"capture","title":"Search reconstruction","description":"Interpretation of the lecture transcript.","sourceIds":["l1:t0"],"uncertainties":["Exact original labels remain unclear."],"diagram":diagram});
        let before = all_evidence(store, "course").unwrap();
        let mut bad = input.clone();
        bad["sourceIds"] = json!(["l1:c:capture"]);
        assert!(visuals::save(store, bad).unwrap_err().contains("support"));
        let artifact = visuals::save(store, input).unwrap();
        assert_eq!(artifact["format"], "diagram");
        assert_eq!(all_evidence(store, "course").unwrap(), before);
        assert_eq!(
            fs::read(
                store
                    .path("courses/course/lectures/l1/captures/capture.png")
                    .unwrap()
            )
            .unwrap(),
            b"original evidence"
        );
        visuals::review(store,json!({"lectureId":"l1","reviews":[{"captureId":"capture","status":"unclear","reason":"Small blurry board text"}]})).unwrap();
        assert_eq!(
            store.read().unwrap()["lectures"][1]["captures"][0]["readability"]["status"],
            "unclear"
        );
    }

    #[test]
    fn image_reconstructions_normalize_jpeg_to_png_and_reject_bad_pixels() {
        use base64::Engine;
        let f = fixture();
        let store = &f.0;
        store.mutate(|s|{s["lectures"][1]["captures"]=json!([{"id":"capture","file":"courses/course/lectures/l1/captures/capture.png","seconds":5,"kind":"whiteboard","stream":"Board","caption":"Search illustration"}]);Ok(())}).unwrap();
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(32, 32)
            .write_to(&mut bytes, image::ImageFormat::Jpeg)
            .unwrap();
        let image = format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
        );
        let input = json!({"lectureId":"l1","captureId":"capture","title":"Reconstruction","description":"Derived from original transcript evidence.","sourceIds":["l1:t0"],"uncertainties":["Exact board details remain uncertain."],"image":image});
        let artifact = visuals::save(store, input.clone()).unwrap();
        let bytes = fs::read(store.path(text(&artifact, "file")).unwrap()).unwrap();
        assert_eq!(
            image::guess_format(&bytes).unwrap(),
            image::ImageFormat::Png
        );
        let listed = call(
            store,
            "get_capture_artifacts",
            json!({"lectureId":"l1","captureId":"capture"}),
        )
        .unwrap();
        assert_eq!(list(&listed, "artifacts").len(), 1);
        let mut invalid = input;
        invalid["image"] = json!("data:image/jpeg;base64,aW52YWxpZA==");
        assert!(visuals::save(store, invalid).is_err());
    }

    #[test]
    fn lecture_preparation_and_task_workflow_do_not_enqueue_or_modify_data() {
        let f = fixture();
        let store = &f.0;
        let before = store.read().unwrap();
        let prepared = call(
            store,
            "prepare_lecture_agent",
            json!({"courseId":"course","lectureId":"l1"}),
        )
        .unwrap();
        assert_eq!(prepared["status"], "prepared_not_spawned");
        assert_eq!(prepared["spawnArguments"]["model"], "gpt-6-astra");
        assert!(
            prepared["spawnArguments"]["message"]
                .as_str()
                .unwrap()
                .contains("Do not delegate")
        );
        let workflow = call(
            store,
            "get_task_workflow",
            json!({"courseId":"course","request":"Check assignments"}),
        )
        .unwrap();
        assert_eq!(workflow["sources"]["canvas"], "https://example.edu/course");
        assert_eq!(workflow["sources"]["website"], Value::Null);
        assert_eq!(store.read().unwrap(), before);
    }
}
