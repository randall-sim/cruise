use crate::store::{Result, *};
use base64::{Engine, engine::general_purpose::STANDARD};
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Cursor,
};

pub fn text(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").into()
}
pub fn string(v: &Value, k: &str, min: usize, max: usize, default: Option<&str>) -> Result<String> {
    let s = match v.get(k) {
        Some(Value::String(s)) => s.trim(),
        None => default.ok_or_else(|| format!("{k} is required"))?,
        _ => return Err(format!("{k}: expected a string")),
    };
    if s.chars().count() < min || s.chars().count() > max {
        return Err(format!("{k}: invalid length"));
    }
    Ok(s.into())
}
fn web_url(v: &Value, k: &str) -> Result<String> {
    let s = string(v, k, 0, 8192, Some(""))?;
    if !s.is_empty() {
        let u = url::Url::parse(&s).map_err(|_| "Use an https:// URL")?;
        if u.scheme() != "https" || u.host_str().is_none() {
            return Err("Use an https:// URL".into());
        }
    }
    Ok(s)
}
fn course_fields(v: &Value) -> Result<Value> {
    let color = string(v, "color", 1, 10, Some("green"))?;
    if !["green", "orange", "blue", "purple"].contains(&color.as_str()) {
        return Err("Invalid course color".into());
    }
    let mut fields = json!({"code":string(v,"code",1,30,None)?,"name":string(v,"name",1,150,None)?,"term":string(v,"term",0,60,Some(""))?,"color":color,"canvasUrl":web_url(v,"canvasUrl")?,"websiteUrl":web_url(v,"websiteUrl")?});
    if v.get("notionUrl").is_some() {
        fields["notionUrl"] = json!(web_url(v, "notionUrl")?);
    }
    Ok(fields)
}
pub fn list_semesters(state: &Value) -> Vec<Value> {
    let mut all = vec![json!({"season":"Fall","year":2026})];
    all.extend(array(state, "semesters").iter().cloned());
    let re = Regex::new(r"(?i)^(Spring|Summer|Fall)\s+(\d{4})$").unwrap();
    for c in array(state, "courses") {
        if let Some(m) = re.captures(text(c, "term").trim()) {
            let year = m[2].parse::<u64>().unwrap_or(0);
            if year >= 1900 {
                let season = match m[1].to_lowercase().as_str() {
                    "spring" => "Spring",
                    "summer" => "Summer",
                    _ => "Fall",
                };
                all.push(json!({"season":season,"year":year}));
            }
        }
    }
    let mut seen = HashSet::new();
    all.retain(|s| seen.insert(format!("{} {}", text(s, "season"), s["year"])));
    all.sort_by_key(|s| {
        std::cmp::Reverse((
            s["year"].as_u64().unwrap_or(0),
            match s["season"].as_str() {
                Some("Fall") => 2,
                Some("Summer") => 1,
                _ => 0,
            },
        ))
    });
    all
}
pub fn timestamp(n: f64) -> String {
    let n = n.floor().max(0.) as u64;
    if n >= 3600 {
        format!("{}:{:02}:{:02}", n / 3600, n / 60 % 60, n % 60)
    } else {
        format!("{:02}:{:02}", n / 60, n % 60)
    }
}
pub fn parse_transcript(raw: &str) -> Result<Vec<Value>> {
    let re = Regex::new(
        r"((?:\d{1,3}:)?\d{2}:\d{2}[.,]\d{3})\s*-->\s*((?:\d{1,3}:)?\d{2}:\d{2}[.,]\d{3})",
    )
    .unwrap();
    let tags = Regex::new(r"<[^>]*>").unwrap();
    let raw = raw.replace('\r', "");
    let lines: Vec<&str> = raw.trim_start_matches('\u{feff}').split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(m) = re.captures(lines[i]) {
            let number = |s: &str| -> f64 {
                s.replace(',', ".")
                    .split(':')
                    .fold(0., |v, p| v * 60. + p.parse::<f64>().unwrap_or(0.))
            };
            let start = number(&m[1]);
            let end = number(&m[2]);
            if end < start {
                return Err("A transcript cue ends before it starts".into());
            }
            i += 1;
            let mut parts = Vec::new();
            while i < lines.len() && !lines[i].trim().is_empty() {
                parts.push(lines[i]);
                i += 1;
            }
            let cleaned = tags
                .replace_all(&parts.join(" "), "")
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .trim()
                .to_string();
            if !cleaned.is_empty() {
                out.push(json!({"start":start,"end":end,"text":cleaned}));
            }
        }
        i += 1;
    }
    if out.is_empty() {
        return Err(
            "No timestamped cues found. Import a WebVTT (.vtt) or SubRip (.srt) transcript.".into(),
        );
    }
    if out.len() > 30000 {
        return Err("Transcript exceeds 30,000 cues".into());
    }
    out.sort_by(|a, b| {
        a["start"]
            .as_f64()
            .unwrap()
            .total_cmp(&b["start"].as_f64().unwrap())
    });
    Ok(out)
}
pub fn normalize_image(data: &str, width: u32, pixel_limit: u64) -> Result<Vec<u8>> {
    let (header, b64) = data
        .split_once(',')
        .ok_or("Only PNG, JPEG, and WebP captures are accepted")?;
    if ![
        "data:image/png;base64",
        "data:image/jpeg;base64",
        "data:image/webp;base64",
    ]
    .contains(&header)
        || data.len() > 16_000_000
    {
        return Err("Only PNG, JPEG, and WebP captures are accepted (maximum 16 MB)".into());
    }
    let bytes = STANDARD.decode(b64).map_err(|e| e.to_string())?;
    let mut reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let (w, h) = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    if u64::from(w) * u64::from(h) > pixel_limit {
        return Err("Image exceeds pixel limit".into());
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(pixel_limit * 8);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?;
    let img = if w > width {
        img.resize(
            width,
            ((h as f64 * width as f64 / w as f64).round() as u32).max(1),
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };
    let mut png = Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(png.into_inner())
}
pub fn handles(name: &str) -> bool {
    [
        "get_workspace_overview",
        "list_courses",
        "list_lectures",
        "list_semester_workspaces",
        "create_semester_workspace",
        "lecture.import",
        "create_semester",
        "create_course",
        "update_course",
        "get_course",
        "import_lecture",
        "get_lecture",
        "save_capture",
        "save_lecture_capture",
        "set_lecture_coverage",
        "save_course_source",
        "search_course",
        "get_course_file_index",
        "read_course_context",
        "course.delete",
        "lecture.hide",
        "lecture.review",
        "concept.toggle",
    ]
    .contains(&name)
}
pub fn call(store: &Store, name: &str, v: Value) -> Result<Value> {
    match name {
        "list_courses" => Ok(store.read()?["courses"].clone()),
        "list_semester_workspaces" => Ok(json!(list_semesters(&store.read()?))),
        "list_lectures" => {
            let cid = arg(&v, "courseId")?;
            let s = store.read()?;
            require_course(&s, cid)?;
            let mut ls = array(&s, "lectures")
                .iter()
                .filter(|l| l["courseId"] == cid)
                .cloned()
                .collect::<Vec<_>>();
            ls.sort_by_key(|l| (text(l, "date"), text(l, "id")));
            let offset = v["offset"].as_u64().unwrap_or(0) as usize;
            let limit = v["limit"].as_u64().unwrap_or(25).clamp(1, 100) as usize;
            Ok(
                json!({"lectures":ls.iter().skip(offset).take(limit).map(|l|json!({"id":l["id"],"courseId":cid,"title":l["title"],"date":l["date"],"sourceUrl":l["sourceUrl"],"status":l["status"]})).collect::<Vec<_>>(),"total":ls.len(),"nextOffset":if offset+limit<ls.len(){json!(offset+limit)}else{Value::Null}}),
            )
        }
        "get_workspace_overview" => {
            let state = store.read()?;
            let courses = array(&state, "courses")
                .iter()
                .map(|course| {
                    let mut course = course.clone();
                    course["lectures"] = json!(
                        array(&state, "lectures")
                            .iter()
                            .filter(|lecture| lecture["courseId"] == course["id"])
                            .count()
                    );
                    course
                })
                .collect::<Vec<_>>();
            let jobs = array(&state, "jobs")
                .iter()
                .filter(|job| job["status"] != "completed")
                .map(|job| {
                    let mut job = job.clone();
                    job["evidenceCount"] = json!(array(&job, "context").len());
                    for key in ["context", "result", "exam", "examRequest"] {
                        job.as_object_mut().unwrap().remove(key);
                    }
                    job
                })
                .collect::<Vec<_>>();
            Ok(
                json!({"semesters":list_semesters(&state),"courses":courses,"jobs":jobs,
            "taskWorkflow":"Use get_task_workflow, then browser computer use: Canvas/course website for requirements; workspace checkpoints for local progress.",
            "policy":crate::study::POLICY}),
            )
        }
        "create_semester" | "create_semester_workspace" => {
            let season = arg(&v, "season")?;
            let year = v["year"].as_u64().ok_or("Invalid semester year")?;
            if !["Spring", "Summer", "Fall"].contains(&season) || !(1900..=9999).contains(&year) {
                return Err("Invalid semester".into());
            }
            let sem = json!({"season":season,"year":year});
            store.mutate(|s| {
                if list_semesters(s).contains(&sem) {
                    return Err("This semester workspace already exists".into());
                }
                array_mut(s, "semesters")?.push(sem.clone());
                Ok(sem)
            })
        }
        "create_course" => {
            let mut course = course_fields(&v)?;
            course["id"] = json!(id());
            course["createdAt"] = json!(now());
            store.mutate(|s|{let base=format!("courses/{}",text(&course,"id"));store.write(&format!("{base}/course.json"),&serde_json::to_vec_pretty(&course).unwrap())?;store.write(&format!("{base}/memory/overview.md"),format!("# {} — {}\n\nTerm: {}\n\nAdd course-level context here. Lecture transcripts are indexed separately.\n",text(&course,"code"),text(&course,"name"),text(&course,"term")).as_bytes())?;store.write(&format!("{base}/AGENTS.md"),format!("# {} course workspace\n\n{}",text(&course,"code"),include_str!("course_instructions.md")).as_bytes())?;array_mut(s,"courses")?.push(course.clone());Ok(course)})
        }
        "update_course" => {
            let cid = arg(&v, "courseId").or_else(|_| arg(&v, "id"))?;
            store.mutate(|s| {
                let old = require_course(s, cid)?;
                let mut combined = old.clone();
                let fields = v.get("changes").or_else(|| v.get("patch")).unwrap_or(&v);
                for (k, val) in fields.as_object().ok_or("Expected course fields")? {
                    if [
                        "code",
                        "name",
                        "term",
                        "color",
                        "canvasUrl",
                        "websiteUrl",
                        "notionUrl",
                    ]
                    .contains(&k.as_str())
                    {
                        combined[k] = val.clone();
                    }
                }
                let parsed = course_fields(&combined)?;
                for (k, val) in parsed.as_object().unwrap() {
                    combined[k] = val.clone();
                }
                let entry = array_mut(s, "courses")?
                    .iter_mut()
                    .find(|c| c["id"] == cid)
                    .unwrap();
                *entry = combined.clone();
                store.write(
                    &format!("courses/{cid}/course.json"),
                    &serde_json::to_vec_pretty(&combined).unwrap(),
                )?;
                Ok(combined)
            })
        }
        "course.delete" => {
            let cid = arg(&v, "id")?;
            store.mutate(|s| {
                let c = require_course(s, cid)?;
                let recovery = format!("recovery/courses/{cid}-{}.json", id());
                let mut backup = json!({"deletedAt":now(),"course":c});
                for key in ["lectures", "tasks", "concepts", "jobs"] {
                    backup[key] = json!(
                        array(s, key)
                            .iter()
                            .filter(|x| x["courseId"] == cid)
                            .collect::<Vec<_>>()
                    );
                }
                store.write(&recovery, &serde_json::to_vec_pretty(&backup).unwrap())?;
                array_mut(s, "courses")?.retain(|c| c["id"] != cid);
                for k in ["lectures", "tasks", "concepts", "jobs"] {
                    array_mut(s, k)?.retain(|c| c["courseId"] != cid);
                }
                Ok(json!({"courseId":cid,"recoveryPath":recovery}))
            })
        }
        "get_course" => {
            let cid = arg(&v, "courseId")?;
            let s = store.read()?;
            let c = require_course(&s, cid)?;
            let ls = array(&s, "lectures")
                .iter()
                .filter(|l| l["courseId"] == cid)
                .map(|l| {
                    let mut l = l.clone();
                    l["cueCount"] = json!(array(&l, "cues").len());
                    l["captureCount"] = json!(array(&l, "captures").len());
                    for k in ["cues", "captures", "guide", "evidence"] {
                        l.as_object_mut().unwrap().remove(k);
                    }
                    l
                })
                .collect::<Vec<_>>();
            let jobs = array(&s, "jobs")
                .iter()
                .filter(|j| j["courseId"] == cid)
                .map(|j| {
                    let mut j = j.clone();
                    for k in ["context", "result", "exam", "examRequest"] {
                        j.as_object_mut().unwrap().remove(k);
                    }
                    j
                })
                .collect::<Vec<_>>();
            Ok(
                json!({"course":c,"lectures":ls,"concepts":array(&s,"concepts").iter().filter(|c|c["courseId"]==cid).collect::<Vec<_>>(),"jobs":jobs}),
            )
        }
        "import_lecture" | "lecture.import" => {
            let cid = arg(&v, "courseId")?;
            valid_id(cid)?;
            let title = string(&v, "title", 1, 200, None)?;
            let date = string(&v, "date", 10, 10, None)?;
            if !Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap().is_match(&date) {
                return Err("Invalid lecture date".into());
            }
            let source = web_url(&v, "sourceUrl")?;
            let transcript = arg(&v, "transcript")?;
            if transcript.len() > 2_000_000 {
                return Err("Transcript too large".into());
            }
            let cues = parse_transcript(transcript)?;
            let coverage = string(
                &v,
                "captureCoverage",
                0,
                3000,
                Some("Transcript only. Visual material has not been captured."),
            )?;
            store.mutate(|s| {
                require_course(s, cid)?;
                let lid = id();
                let base = format!("courses/{cid}/lectures/{lid}");
                let duration = cues.iter().filter_map(|cue| cue["end"].as_f64()).fold(0., f64::max);
                let lecture = json!({
                    "id": lid, "courseId": cid, "title": title, "date": date,
                    "sourceUrl": source, "createdAt": now(), "cues": cues,
                    "captures": [], "duration": duration, "status": "imported",
                    "notesPath": format!("{base}/notes.md"),
                    "transcriptPath": format!("{base}/transcript.md"),
                    "captureCoverage": coverage, "reviewed": false
                });
                store.write(&format!("{base}/source.vtt"), transcript.as_bytes())?;
                let lines = cues.iter().enumerate().map(|(index, cue)| {
                    format!("## {} [{lid}:t{index}]\n\n{}",
                        timestamp(cue["start"].as_f64().unwrap_or(0.)), text(cue, "text"))
                }).collect::<Vec<_>>().join("\n\n");
                store.write(
                    &format!("{base}/transcript.md"),
                    format!("# {title}\n\nSource: {}\n\n{lines}",
                        if source.is_empty() { "Uploaded transcript" } else { &source }).as_bytes()
                )?;
                store.write(
                    &format!("{base}/notes.md"),
                    format!("# {title}\n\nEvidence imported; the complete lecture guide is not complete. Continue visual capture and inspection, then create and complete a lecture job.\n\nCapture coverage: {coverage}\n").as_bytes()
                )?;
                array_mut(s, "lectures")?.push(lecture.clone());
                Ok(if name == "import_lecture" {
                    json!({
                        "id": lecture["id"], "courseId": lecture["courseId"],
                        "cues": array(&lecture, "cues").len(), "duration": lecture["duration"],
                        "status": "imported_not_summarized",
                        "nextStep": "Discover and save relevant Canvas/course-site resources, inspect deck/video images, then queue_study_job(kind=lecture), read every evidence page and complete_job. Use and link the resources in the guide. The complete lecture guide is not finished yet."
                    })
                } else {
                    lecture
                })
            })
        }
        "get_lecture" => {
            let s = store.read()?;
            let cid = arg(&v, "courseId")?;
            require_course(&s, cid)?;
            let lid = arg(&v, "lectureId")?;
            let mut l = array(&s, "lectures")
                .iter()
                .find(|l| l["id"] == lid && l["courseId"] == cid)
                .cloned()
                .ok_or("Lecture not found in this course")?;
            let offset = v["offset"].as_u64().unwrap_or(0) as usize;
            let limit = v["limit"].as_u64().unwrap_or(50).clamp(1, 100) as usize;
            let cues = array(&l, "cues").to_vec();
            l["cues"] = json!(cues.iter().skip(offset).take(limit).collect::<Vec<_>>());
            l["total"] = json!(cues.len());
            l["nextOffset"] = if offset + limit < cues.len() {
                json!(offset + limit)
            } else {
                Value::Null
            };
            Ok(l)
        }
        "set_lecture_coverage" | "lecture.hide" | "lecture.review" => store.mutate(|s| {
            let lid = arg(&v, "lectureId").or_else(|_| arg(&v, "id"))?;
            if let Some(c) = v["courseId"].as_str() {
                require_course(s, c)?;
            }
            let l = array_mut(s, "lectures")?
                .iter_mut()
                .find(|l| {
                    l["id"] == lid
                        && (v.get("courseId").is_none() || l["courseId"] == v["courseId"])
                })
                .ok_or("Lecture not found in this course")?;
            match name {
                "set_lecture_coverage" => {
                    let coverage = string(&v, "coverage", 1, 3000, None)?;
                    l["captureCoverage"] = json!(coverage);
                    Ok(json!({"lectureId":lid,"coverage":coverage}))
                }
                "lecture.hide" => {
                    l["hiddenFromUi"] = json!(true);
                    Ok(json!({"lectureId":lid,"hiddenFromUi":true}))
                }
                _ => {
                    l["reviewed"] =
                        json!(v["reviewed"].as_bool().ok_or("Expected reviewed boolean")?);
                    Ok(Value::Null)
                }
            }
        }),
        "concept.toggle" => store.mutate(|s| {
            let c = array_mut(s, "concepts")?
                .iter_mut()
                .find(|c| c["id"] == v["id"])
                .ok_or("Concept not found")?;
            c["mastered"] = json!(!c["mastered"].as_bool().unwrap_or(false));
            Ok(Value::Null)
        }),
        "save_capture" | "save_lecture_capture" => {
            let lid = arg(&v, "lectureId")?;
            let sec = v["seconds"]
                .as_f64()
                .filter(|n| *n >= 0.)
                .ok_or("Invalid capture timestamp")?;
            let kind = arg(&v, "kind")?;
            if !["slide", "whiteboard", "demo"].contains(&kind) {
                return Err("Invalid capture kind".into());
            }
            let stream = string(&v, "stream", 1, 120, None)?;
            let caption = string(&v, "caption", 0, 3000, Some(""))?;
            let png = normalize_image(arg(&v, "image")?, 1920, 20_000_000)?;
            store.mutate(|s| {
                let lecture = array_mut(s, "lectures")?
                    .iter_mut()
                    .find(|lecture| lecture["id"] == lid)
                    .ok_or("Lecture not found")?;
                if sec > lecture["duration"].as_f64().unwrap_or(0.) + 60. {
                    return Err("Capture timestamp is outside the lecture".into());
                }
                if array(lecture, "captures").len() >= 1000 {
                    return Err("Maximum of 1,000 captures per lecture".into());
                }
                let id = id();
                let file = format!("courses/{}/lectures/{lid}/captures/{id}.png", text(lecture, "courseId"));
                let capture = json!({"id":id,"seconds":sec,"kind":kind,"stream":stream,"caption":caption,"file":file});
                store.write(&file, &png)?;
                let captures = array_mut(lecture, "captures")?;
                captures.push(capture.clone());
                captures.sort_by(|a, b| a["seconds"].as_f64().unwrap_or(0.).total_cmp(&b["seconds"].as_f64().unwrap_or(0.)));
                Ok(capture)
            })
        }
        "save_course_source" => {
            let cid = arg(&v, "courseId")?;
            let title = string(&v, "title", 1, 200, None)?;
            let url = web_url(&v, "url")?;
            let content = string(&v, "text", 10, 300000, None)?;
            store.mutate(|s| {
                require_course(s, cid)?;
                let sid = id();
                let file = format!("courses/{cid}/memory/{sid}.md");
                store.write(
                    &file,
                    format!(
                        "# {title}\n\nImported: {}\nSource: {}\n\n{content}\n",
                        now(),
                        if url.is_empty() {
                            "User-provided course material"
                        } else {
                            &url
                        }
                    )
                    .as_bytes(),
                )?;
                Ok(json!({"id":sid,"path":file}))
            })
        }
        "search_course" => {
            let cid = arg(&v, "courseId")?;
            let query = string(&v, "query", 1, 3000, None)?;
            let limit = v["limit"].as_u64().unwrap_or(8).min(50) as usize;
            Ok(json!(rank_evidence(
                all_evidence(store, cid)?,
                &query,
                limit
            )))
        }
        "get_course_file_index" => {
            let mut index = index_files(store, arg(&v, "courseId")?)?;
            let evidence = index["evidence"].as_array().unwrap().clone();
            let offset = v["offset"].as_u64().unwrap_or(0) as usize;
            let limit = v["limit"].as_u64().unwrap_or(20).clamp(1, 50) as usize;
            index["evidence"] = json!(evidence.iter().skip(offset).take(limit).collect::<Vec<_>>());
            index["total"] = json!(evidence.len());
            index["nextOffset"] = if offset + limit < evidence.len() {
                json!(offset + limit)
            } else {
                Value::Null
            };
            Ok(index)
        }
        "read_course_context" => crate::context::read(store, v),
        _ => Err(format!("Unknown tool: {name}")),
    }
}

pub fn slice(s: &str, start: usize, len: usize) -> String {
    String::from_utf16_lossy(&s.encode_utf16().skip(start).take(len).collect::<Vec<_>>())
}
pub fn length(s: &str) -> usize {
    s.encode_utf16().count()
}
fn chunks(s: &str) -> Vec<(usize, String)> {
    // Source IDs retain JavaScript UTF-16 offsets across the runtime migration.
    let units: Vec<_> = s.encode_utf16().collect();
    (0..units.len())
        .step_by(5500)
        .map(|i| {
            (
                i,
                String::from_utf16_lossy(&units[i..(i + 6000).min(units.len())]),
            )
        })
        .collect()
}
fn js_whitespace(c: char) -> bool {
    matches!(c,'\u{0009}'..='\u{000d}'|'\u{0020}'|'\u{00a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
fn paragraphs(s: &str) -> Vec<String> {
    // Equivalent to /\n(?=#{1,3} )|\n\s*\n/; retain original excerpt whitespace.
    static BOUNDARY: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let boundary=BOUNDARY.get_or_init(||Regex::new(r"\n(#{1,3} )|\n[\t\n\r\x0b\x0c \u{00a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]*\n").unwrap());
    let mut result = Vec::new();
    let mut start = 0;
    for captures in boundary.captures_iter(s) {
        let m = captures.get(0).unwrap();
        result.push(s[start..m.start()].to_owned());
        start = if captures.get(1).is_some() {
            m.start() + 1
        } else {
            m.end()
        };
    }
    result.push(s[start..].to_owned());
    result
}
fn markdown(
    store: &Store,
    course: &str,
    relative: &str,
    depth: usize,
    out: &mut Vec<Value>,
) -> Result<()> {
    if depth > 4 {
        return Ok(());
    }
    let dir = store.path(relative)?;
    if !dir.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        if name.starts_with('.') || ty.is_symlink() {
            continue;
        }
        let file = format!("{relative}/{name}");
        if ty.is_dir() {
            markdown(store, course, &file, depth + 1, out)?;
        } else if ty.is_file()
            && name.ends_with(".md")
            && entry.metadata().map_err(|e| e.to_string())?.len() <= 2_000_000
        {
            let bytes = fs::read(store.path(&file)?).map_err(|e| e.to_string())?;
            let content = String::from_utf8_lossy(&bytes);
            let title = content
                .strip_prefix("# ")
                .and_then(|s| s.split(['\r', '\n', '\u{2028}', '\u{2029}']).next())
                .filter(|s| !s.is_empty())
                .unwrap_or(&name);
            let url = content
                .split('\n')
                .find_map(|l| {
                    l.strip_prefix("Source: https://")
                        .and_then(|s| s.split(js_whitespace).next())
                        .filter(|s| !s.is_empty())
                })
                .map(|s| format!("https://{s}"));
            let assignment = file.starts_with(&format!("courses/{course}/memory/assignments/"))
                || file.starts_with(&format!("courses/{course}/memory/files/"));
            let mut n = 0;
            for p in paragraphs(&content)
                .iter()
                .filter(|p| length(p.trim_matches(js_whitespace)) > 30)
            {
                for (_, t) in chunks(p) {
                    let mut ev = json!({"id":format!("memory:{file}:{n}"),"courseId":course,"title":title,"text":if assignment{format!("Generated assignment learning/work history; not primary instructor evidence.\n{t}")}else{t},"path":file,"kind":if assignment{"assignment"}else{"note"}});
                    if let Some(url) = &url {
                        ev["url"] = json!(url);
                    }
                    out.push(ev);
                    n += 1;
                }
            }
        }
    }
    Ok(())
}
pub fn excluded(path: &str) -> bool {
    path.split('/').any(|p| {
        [
            ".git",
            ".hg",
            ".svn",
            "node_modules",
            ".venv",
            "venv",
            "__pycache__",
            ".next",
            "dist",
            "build",
            "target",
            "coverage",
            ".cache",
            ".runtime",
            ".secrets",
            ".ssh",
            ".npmrc",
            ".pypirc",
            ".netrc",
            ".git-credentials",
            "id_rsa",
            "id_ed25519",
        ]
        .contains(&p)
            || p == ".env"
            || p.starts_with(".env.")
            || ["auth.json", "storage-state.json"].contains(&p.to_lowercase().as_str())
            || [".pem", ".key", ".p12", ".pfx"]
                .iter()
                .any(|s| p.to_lowercase().ends_with(s))
    })
}
pub fn index_files(store: &Store, course: &str) -> Result<Value> {
    require_course(&store.read()?, course)?;
    let mut ctx = Index {
        store,
        course,
        files: 0,
        bytes: 0,
        visited: 0,
        evidence: Vec::new(),
        skipped: Vec::new(),
    };
    ctx.walk("", 0)?;
    Ok(
        json!({"evidence":ctx.evidence,"files":ctx.files,"bytes":ctx.bytes,"skipped":ctx.skipped,"excluded":"VCS metadata, dependencies, build output, caches and credential files","provenance":"Local working files, including imported and generated code; not automatically instructor authority. Source IDs include a content revision."}),
    )
}
struct Index<'a> {
    store: &'a Store,
    course: &'a str,
    files: usize,
    bytes: u64,
    visited: usize,
    evidence: Vec<Value>,
    skipped: Vec<Value>,
}
impl Index<'_> {
    fn walk(&mut self, relative: &str, depth: usize) -> Result<()> {
        let base = format!("courses/{}/files", self.course);
        let dir = self.store.path(&if relative.is_empty() {
            base.clone()
        } else {
            format!("{base}/{relative}")
        })?;
        if !dir.exists() {
            return Ok(());
        }
        let mut entries = fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let leaf = e.file_name().to_string_lossy().into_owned();
            let name = if relative.is_empty() {
                leaf
            } else {
                format!("{relative}/{leaf}")
            };
            if excluded(&name) {
                continue;
            }
            let ty = e.file_type().map_err(|e| e.to_string())?;
            if ty.is_symlink() {
                self.skipped.push(json!({"path":name,"reason":"Symlink"}));
                continue;
            }
            self.visited += 1;
            if self.visited > 20000 || depth > 20 {
                self.skipped
                    .push(json!({"path":name,"reason":"Index traversal limit"}));
                return Ok(());
            }
            if ty.is_dir() {
                self.walk(&name, depth + 1)?;
                continue;
            }
            if !ty.is_file() {
                continue;
            }
            let file = format!("{base}/{name}");
            let p = self.store.path(&file)?;
            let size = fs::metadata(&p).map_err(|e| e.to_string())?.len();
            if size > 2_000_000 || self.bytes + size > 30_000_000 {
                self.skipped.push(
                    json!({"path":name,"reason":"Text index size limit (2 MB/file, 30 MB/course)"}),
                );
                continue;
            }
            let bytes = fs::read(p).map_err(|e| e.to_string())?;
            let content = match std::str::from_utf8(&bytes) {
                Ok(s) if !s.contains('\0') => s.strip_prefix('\u{feff}').unwrap_or(s),
                _ => {
                    self.skipped.push(json!({"path":name,"reason":"Binary; add a Markdown description or extracted text"}));
                    continue;
                }
            };
            if content.trim_matches(js_whitespace).is_empty() {
                continue;
            }
            self.bytes += bytes.len() as u64;
            self.files += 1;
            let version = format!("{:x}", Sha256::digest(&bytes));
            let units: Vec<_> = content.encode_utf16().collect();
            let mut line = 1;
            let mut previous = 0;
            for (start, chunk) in chunks(content) {
                line += units[previous..start].iter().filter(|&&c| c == 10).count();
                previous = start;
                let query = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("course", self.course)
                    .append_pair("tab", "files")
                    .append_pair("file", &name)
                    .finish();
                self.evidence.push(json!({"id":format!("file:{file}:{}:{start}",&version[..16]),"courseId":self.course,"kind":"file","title":format!("{name} · line {line}"),"text":chunk,"path":file,"url":format!("/?{query}")}));
            }
        }
        Ok(())
    }
}
pub fn evidence(store: &Store, state: &Value, course: &str) -> Result<Vec<Value>> {
    require_course(state, course)?;
    let mut out = Vec::new();
    for l in array(state, "lectures")
        .iter()
        .filter(|l| l["courseId"] == course)
    {
        let lid = text(l, "id");
        for (i, c) in array(l, "cues").iter().enumerate() {
            out.push(json!({"id":format!("{lid}:t{i}"),"courseId":course,"lectureId":lid,"title":l["title"],"seconds":c["start"],"text":c["text"],"path":l["transcriptPath"],"url":l["sourceUrl"],"kind":"transcript"}));
        }
        for c in array(l, "captures") {
            out.push(json!({"id":format!("{lid}:c:{}",text(c,"id")),"courseId":course,"lectureId":lid,"title":format!("{} · {} · {}",text(l,"title"),text(c,"stream"),text(c,"kind")),"seconds":c["seconds"],"text":if text(c,"caption").is_empty(){format!("Uncaptioned {}. Inspect the image before describing it.",text(c,"kind"))}else{text(c,"caption")},"path":c["file"],"kind":"capture"}));
        }
    }
    for t in array(state, "tasks")
        .iter()
        .filter(|t| t["courseId"] == course && t["archived"] != true)
    {
        out.push(json!({"id":format!("task:{}",text(t,"id")),"courseId":course,"title":t["title"],"text":format!("Historical task snapshot — not current requirements or status. Recheck official Canvas/course websites; use workspace checkpoints for local progress.\n{}\n{}\nPreviously recorded due date: {}",text(t,"title"),text(t,"description"),t["due"].as_str().unwrap_or("not specified")),"path":"state.json","url":t["url"],"kind":"assignment"}));
    }
    markdown(
        store,
        course,
        &format!("courses/{course}/memory"),
        0,
        &mut out,
    )?;
    out.extend(
        array(&index_files(store, course)?, "evidence")
            .iter()
            .cloned(),
    );
    Ok(out)
}
pub fn all_evidence(store: &Store, course: &str) -> Result<Vec<Value>> {
    evidence(store, &store.read()?, course)
}
pub fn rank_evidence(sources: Vec<Value>, query: &str, limit: usize) -> Vec<Value> {
    static WORDS: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let pattern = WORDS.get_or_init(|| Regex::new(r"[\p{L}\p{N}]{2,}").unwrap());
    let words = |s: &str| -> Vec<String> {
        pattern
            .find_iter(&s.to_lowercase())
            .map(|m| m.as_str().to_owned())
            .collect()
    };
    let stop:HashSet<&str>="the a an is are was were what how why can could do does of to in it this that and or for with from lecture explain me my about".split_whitespace().collect();
    let mut seen = HashSet::new();
    let terms: Vec<String> = words(query)
        .into_iter()
        .filter(|w| !stop.contains(w.as_str()) && seen.insert(w.clone()))
        .collect();
    if terms.is_empty() || sources.is_empty() {
        return vec![];
    }
    let docs: Vec<Vec<String>> = sources
        .iter()
        .map(|s| words(&format!("{} {}", text(s, "title"), text(s, "text"))))
        .collect();
    let avg = docs.iter().map(Vec::len).sum::<usize>() as f64 / docs.len() as f64;
    if avg == 0.0 {
        return vec![];
    }
    let frequency: HashMap<&str, usize> = terms
        .iter()
        .map(|t| (t.as_str(), docs.iter().filter(|d| d.contains(t)).count()))
        .collect();
    let mut scored: Vec<(Value, f64)> = sources
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let score = terms
                .iter()
                .map(|t| {
                    let tf = docs[i].iter().filter(|w| *w == t).count() as f64;
                    let df = frequency[t.as_str()] as f64;
                    let idf = (1. + (docs.len() as f64 - df + 0.5) / (df + 0.5)).ln();
                    idf * ((tf * 2.2) / (tf + 1.2 * (0.25 + 0.75 * docs[i].len() as f64 / avg)))
                })
                .sum();
            (s, score)
        })
        .filter(|(_, s)| *s > 0.)
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.into_iter().take(limit).map(|(s, _)| s).collect()
}

#[cfg(test)]
mod evidence_tests {
    use super::*;
    struct Fixture(Store);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.root);
        }
    }
    fn fixture() -> Fixture {
        let store = Store::new(std::env::temp_dir().join(format!("cruise-evidence-{}", id())));
        store
            .mutate(|s| {
                s["courses"] = json!([{"id":"course"},{"id":"other"}]);
                Ok(())
            })
            .unwrap();
        Fixture(store)
    }

    #[test]
    fn markdown_ids_and_original_whitespace_match_typescript_utf16_semantics() {
        let f = fixture();
        let content = format!(
            "# Title\n\nSource: https://example.edu/notes trailing note\n\n{}\n## Third header\nLast paragraph includes enough words to be indexed.\n",
            "😀".repeat(16)
        );
        f.0.write("courses/course/memory/reference.md", content.as_bytes())
            .unwrap();
        let evidence = all_evidence(&f.0, "course").unwrap();
        let expected = [
            "Source: https://example.edu/notes trailing note".to_owned(),
            "😀".repeat(16),
            "## Third header\nLast paragraph includes enough words to be indexed.\n".to_owned(),
        ];
        assert_eq!(evidence.len(), 3);
        for (i, (source, content)) in evidence.iter().zip(expected).enumerate() {
            assert_eq!(
                source["id"],
                format!("memory:courses/course/memory/reference.md:{i}")
            );
            assert_eq!(source["text"], content);
            assert_eq!(source["url"], "https://example.edu/notes");
            assert_eq!(source["title"], "Title");
        }
        assert_eq!(paragraphs("a\n\u{feff}\n b\n"), vec!["a", " b\n"]);
    }

    #[test]
    fn file_evidence_keeps_revision_hash_utf16_offsets_bom_decoding_and_line_numbers() {
        let f = fixture();
        let raw = format!(
            "\u{feff}header\n{}{}\n{}",
            "😀".repeat(100),
            "x".repeat(5293),
            "tail ".repeat(200)
        );
        f.0.write("courses/course/files/unicode.txt", raw.as_bytes())
            .unwrap();
        let index = index_files(&f.0, "course").unwrap();
        let sources = array(&index, "evidence");
        assert_eq!(index["files"], 1);
        assert_eq!(index["bytes"], 6704);
        assert_eq!(sources.len(), 2);
        // Golden hash, offsets and lengths from the original Node TextDecoder/SHA-256 indexer.
        assert_eq!(
            sources[0]["id"],
            "file:courses/course/files/unicode.txt:d928c05c1cbf09ca:0"
        );
        assert_eq!(
            sources[1]["id"],
            "file:courses/course/files/unicode.txt:d928c05c1cbf09ca:5500"
        );
        assert_eq!(length(sources[0]["text"].as_str().unwrap()), 6000);
        assert_eq!(sources[1]["title"], "unicode.txt · line 2");
        assert_eq!(sources[1]["text"], format!("\n{}", "tail ".repeat(200)));
        f.0.write("courses/course/files/unicode.txt", b"changed revision")
            .unwrap();
        assert_ne!(
            index_files(&f.0, "course").unwrap()["evidence"][0]["id"],
            sources[0]["id"]
        );
    }

    #[test]
    fn retrieval_unicode_tokens_ranking_ties_and_scoping_match_previous_engine() {
        let docs = vec![
            json!({"id":"first","title":"","text":"א\u{05b0}בגד 中文 binary sorted"}),
            json!({"id":"second","title":"","text":"א\u{05b0}בגד 中文 binary sorted"}),
            json!({"id":"third","title":"","text":"Separate topic"}),
        ];
        let ranked = rank_evidence(docs.clone(), "בגד 中文 binary binary the", 8);
        assert_eq!(
            ranked.iter().map(|s| text(s, "id")).collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert!(rank_evidence(docs.clone(), "the of or", 8).is_empty());
        assert!(rank_evidence(docs, "א", 8).is_empty());
        let f = fixture();
        f.0.mutate(|s|{s["lectures"]=json!([{"id":"lecture","courseId":"course","title":"Binary search","sourceUrl":"https://example.edu/video","transcriptPath":"courses/course/lectures/lecture/transcript.md","cues":[{"start":1,"end":10,"text":"Binary search requires a sorted interval."}],"captures":[{"id":"capture","seconds":3,"stream":"Slides","kind":"slide","caption":"","file":"courses/course/lectures/lecture/captures/capture.png"}]},{"id":"private","courseId":"other","cues":[{"start":0,"text":"Binary unrelated private"}],"captures":[]}]);s["tasks"]=json!([{"id":"old","courseId":"course","title":"Past task","description":"Historical requirements","due":null,"url":"https://example.edu/task"},{"id":"hidden","courseId":"course","archived":true}]);Ok(())}).unwrap();
        f.0.write("courses/course/memory/assignments/work.md",b"# Learning\n\nBinary search progress is generated secondary reasoning, not instructor authority.\n").unwrap();
        let evidence = all_evidence(&f.0, "course").unwrap();
        assert!(evidence.iter().all(|s| s["courseId"] == "course"));
        assert_eq!(evidence[0]["id"], "lecture:t0");
        assert_eq!(evidence[0]["seconds"], 1);
        assert_eq!(evidence[1]["id"], "lecture:c:capture");
        assert_eq!(evidence[2]["id"], "task:old");
        assert!(text(&evidence[2], "text").contains("not current requirements or status"));
        assert!(
            text(&evidence[3], "text").starts_with("Generated assignment learning/work history")
        );
        assert!(!evidence.iter().any(|s| s["id"] == "task:hidden"));
    }

    #[test]
    fn file_index_reports_binary_and_symlink_omissions_and_excludes_credentials() {
        let f = fixture();
        for (path, data) in [
            ("src/code.py", b"print('retained')".as_slice()),
            ("node_modules/package/data.txt", b"dependency secret"),
            (".env", b"TOKEN=secret"),
            ("AUTH.JSON", b"credential secret"),
            ("binary.bin", &[0, 255]),
        ] {
            f.0.write(&format!("courses/course/files/{path}"), data)
                .unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            std::env::temp_dir(),
            f.0.root.join("courses/course/files/link"),
        )
        .unwrap();
        let indexed = index_files(&f.0, "course").unwrap();
        assert_eq!(indexed["files"], 1);
        assert!(!indexed["evidence"].to_string().contains("secret"));
        assert!(
            array(&indexed, "skipped")
                .iter()
                .any(|s| s["path"] == "binary.bin")
        );
        #[cfg(unix)]
        assert!(
            array(&indexed, "skipped")
                .iter()
                .any(|s| s["path"] == "link" && s["reason"] == "Symlink")
        );
    }
}
