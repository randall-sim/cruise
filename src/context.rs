use crate::{
    core::{all_evidence, length, rank_evidence, slice, text, timestamp},
    store::*,
};
use serde_json::{Value, json};
use std::{collections::HashSet, fs};

fn link(pairs: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        if !v.is_empty() {
            s.append_pair(k, v);
        }
    }
    let q = s.finish();
    if q.is_empty() {
        "/context".into()
    } else {
        format!("/context?{q}")
    }
}
fn reference(source: &Value) -> Value {
    let mut v = source.clone();
    v["contextUrl"] = json!(link(&[
        ("courseId", &text(source, "courseId")),
        ("sourceId", &text(source, "id"))
    ]));
    if source["kind"] == "capture" {
        v["imageUrl"] = json!(format!(
            "/api/capture/{}",
            text(source, "id").split(":c:").nth(1).unwrap_or("")
        ));
    }
    v
}
fn next(start: usize, size: usize, total: usize) -> Value {
    if start + size < total {
        json!(start + size)
    } else {
        Value::Null
    }
}

pub fn read(store: &Store, v: Value) -> Result<Value> {
    let o = v.as_object().ok_or("Invalid context request")?;
    if o.keys().any(|k| {
        ![
            "courseId",
            "lectureId",
            "sourceId",
            "section",
            "query",
            "offset",
        ]
        .contains(&k.as_str())
    }) {
        return Err("Unknown context parameter".into());
    }
    let course = text(&v, "courseId");
    let lecture = text(&v, "lectureId");
    let source = text(&v, "sourceId");
    let query = text(&v, "query").trim().to_string();
    let section = text(&v, "section");
    if course.is_empty() && (!lecture.is_empty() || !source.is_empty() || !query.is_empty()) {
        return Err("Select a course first".into());
    }
    if !section.is_empty()
        && (lecture.is_empty() || !["guide", "sources"].contains(&section.as_str()))
    {
        return Err("Sections require a lecture".into());
    }
    if [&lecture, &source, &query]
        .iter()
        .filter(|s| !s.is_empty())
        .count()
        > 1
    {
        return Err("Choose a lecture, source, or search query".into());
    }
    if query.len() > 1000 || source.len() > 1000 {
        return Err("Context query too long".into());
    }
    let offset = match v.get("offset") {
        None => 0,
        Some(Value::String(s)) => s.parse::<usize>().map_err(|_| "Invalid offset")?,
        Some(n) => n.as_u64().ok_or("Invalid offset")? as usize,
    };
    if offset > 10_000_000 {
        return Err("Invalid offset".into());
    }
    let state = store.read()?;
    let mut out = json!({"instructions":include_str!("context_instructions.md"),"readOnly":true});
    if course.is_empty() {
        out["kind"] = json!("workspace");
        out["courses"]=json!(array(&state,"courses").iter().skip(offset).take(12).map(|c|json!({"id":c["id"],"code":c["code"],"name":c["name"],"term":c["term"],"contextUrl":link(&[("courseId",&text(c,"id"))])})).collect::<Vec<_>>());
        out["nextOffset"] = next(offset, 12, array(&state, "courses").len());
        return Ok(out);
    }
    let c = require_course(&state, &course)?;
    let mut cb = json!({});
    for k in [
        "id",
        "code",
        "name",
        "term",
        "canvasUrl",
        "websiteUrl",
        "notionUrl",
    ] {
        if let Some(v) = c.get(k) {
            cb[k] = v.clone();
        }
    }
    out["course"] = cb;
    let sources = all_evidence(store, &course)?;
    if !source.is_empty() {
        let ev = sources
            .iter()
            .find(|s| s["id"] == source)
            .ok_or("Source not found in this course")?;
        let kind = text(ev, "kind");
        let path = text(ev, "path");
        let mut document = text(ev, "text");
        if kind == "note"
            || (kind == "assignment"
                && (path.starts_with(&format!("courses/{course}/memory/assignments/"))
                    || path.starts_with(&format!("courses/{course}/memory/files/"))))
        {
            if !path.starts_with(&format!("courses/{course}/memory/")) || !path.ends_with(".md") {
                return Err("Source is not course Markdown memory".into());
            }
            document = fs::read_to_string(store.path(&path)?).map_err(|e| e.to_string())?;
        } else if kind == "file" {
            if !path.starts_with(&format!("courses/{course}/files/")) {
                return Err("Not a course file".into());
            }
            document = fs::read_to_string(store.path(&path)?).map_err(|e| e.to_string())?;
        } else if kind == "transcript" {
            let l = array(&state, "lectures")
                .iter()
                .find(|l| l["id"] == ev["lectureId"] && l["courseId"] == course)
                .ok_or("Lecture not found")?;
            document = array(l, "cues")
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    format!(
                        "[{}:t{i}] {}–{} {}",
                        text(l, "id"),
                        timestamp(c["start"].as_f64().unwrap_or(0.)),
                        timestamp(c["end"].as_f64().unwrap_or(0.)),
                        text(c, "text")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
        }
        let start = if v.get("offset").is_some() {
            offset
        } else {
            let needle = if kind == "transcript" {
                format!("[{source}]")
            } else {
                text(ev,"text").trim_start_matches("Generated assignment learning/work history; not primary instructor evidence.\n").into()
            };
            document
                .find(&needle)
                .map(|i| length(&document[..i]))
                .unwrap_or(0)
        };
        let capture = array(&state, "lectures")
            .iter()
            .filter(|l| l["id"] == ev["lectureId"] && l["courseId"] == course)
            .flat_map(|l| array(l, "captures"))
            .find(|c| format!("{}:c:{}", text(ev, "lectureId"), text(c, "id")) == source);
        let reconstructions=capture.map(|cap|array(cap,"artifacts").iter().map(|a|json!({"id":a["id"],"title":a["title"],"description":a["description"],"format":a["format"],"imageUrl":format!("/api/capture/{}/artifacts/{}",text(cap,"id"),text(a,"id")),"sources":array(a,"sourceIds").iter().map(|id|json!({"id":id,"contextUrl":link(&[("courseId",&course),("sourceId",id.as_str().unwrap_or(""))])})).collect::<Vec<_>>(),"uncertainties":a["uncertainties"],"status":"Generated interpretation, not original lecture evidence"})).collect::<Vec<_>>()).unwrap_or_default();
        out["kind"] = json!("source");
        out["source"] = reference(ev);
        out["reconstructions"] = json!(reconstructions);
        out["documentText"] = json!(slice(&document, start, 12000));
        out["offset"] = json!(start);
        out["totalCharacters"] = json!(length(&document));
        out["nextOffset"] = next(start, 12000, length(&document));
        return Ok(out);
    }
    if !lecture.is_empty() {
        valid_id(&lecture)?;
        let l = array(&state, "lectures")
            .iter()
            .find(|l| l["id"] == lecture && l["courseId"] == course)
            .ok_or("Lecture not found in this course")?;
        let guide = if l.get("guide").is_some() {
            serde_json::to_string_pretty(&l["guide"]).map_err(|e| e.to_string())?
        } else {
            "No generated guide is saved.".into()
        };
        let mut ids = HashSet::new();
        for k in ["sections", "concepts", "logistics", "questions"] {
            for item in array(&l["guide"], k) {
                for id in array(item, "citations") {
                    if let Some(id) = id.as_str() {
                        ids.insert(id.to_string());
                    }
                }
            }
        }
        let mut refs = sources
            .iter()
            .filter(|s| {
                s["lectureId"] == lecture
                    && (s["kind"] == "capture" || s["id"] == format!("{lecture}:t0"))
            })
            .cloned()
            .collect::<Vec<_>>();
        refs.extend(
            sources
                .iter()
                .filter(|s| ids.contains(&text(s, "id")))
                .cloned(),
        );
        let mut seen = HashSet::new();
        refs.retain(|s| seen.insert(text(s, "id")));
        let so = if section == "sources" { offset } else { 0 };
        out["kind"] = json!("lecture");
        out["lecture"] = json!({"id":l["id"],"title":l["title"],"date":l["date"],"sourceUrl":l["sourceUrl"],"captureCoverage":l["captureCoverage"],"hiddenFromUi":l["hiddenFromUi"]==true});
        out["generatedGuide"] = if section == "sources" {
            Value::Null
        } else {
            json!(slice(&guide, offset, 12000))
        };
        out["offset"] = json!(offset);
        out["nextOffset"] = if section == "sources" {
            next(offset, 12, refs.len())
        } else {
            next(offset, 12000, length(&guide))
        };
        out["sources"] = json!(
            refs.iter()
                .skip(so)
                .take(12)
                .map(|s| {
                    let mut r = reference(s);
                    r["text"] = json!(slice(&text(s, "text"), 0, 500));
                    r
                })
                .collect::<Vec<_>>()
        );
        out["totalSources"] = json!(refs.len());
        out["allSourcesUrl"] = json!(link(&[
            ("courseId", &course),
            ("lectureId", &lecture),
            ("section", "sources")
        ]));
        return Ok(out);
    }
    if !query.is_empty() {
        let ranked = rank_evidence(sources, &query, offset + 13);
        out["kind"] = json!("search");
        out["query"] = json!(query);
        out["sources"] = json!(
            ranked
                .iter()
                .skip(offset)
                .take(12)
                .map(|s| {
                    let mut r = reference(s);
                    r["text"] = json!(slice(&text(s, "text"), 0, 1500));
                    r["textTruncated"] = json!(length(&text(s, "text")) > 1500);
                    r
                })
                .collect::<Vec<_>>()
        );
        out["nextOffset"] = next(offset, 12, ranked.len());
        return Ok(out);
    }
    let mut items=array(&state,"lectures").iter().filter(|l|l["courseId"]==course&&l["hiddenFromUi"]!=true).map(|l|json!({"title":l["title"],"detail":format!("{} · lecture",text(l,"date")),"contextUrl":link(&[("courseId",&course),("lectureId",&text(l,"id"))])})).collect::<Vec<_>>();
    let mut paths = Vec::new();
    let mut files = std::collections::HashMap::new();
    for s in &sources {
        let path = text(s, "path");
        if s["kind"] == "note"
            || (s["kind"] == "assignment"
                && (path.starts_with(&format!("courses/{course}/memory/assignments/"))
                    || path.starts_with(&format!("courses/{course}/memory/files/"))))
        {
            if !files.contains_key(&path) {
                paths.push(path.clone());
            }
            files.insert(path, s);
        }
    }
    for path in paths {
        let s = files[&path];
        items.push(json!({"title":s["title"],"detail":path,"contextUrl":link(&[("courseId",&course),("sourceId",&text(s,"id")),("offset","0")])}));
    }
    out["kind"] = json!("course");
    out["items"] = json!(items.iter().skip(offset).take(12).collect::<Vec<_>>());
    out["totalItems"] = json!(items.len());
    out["nextOffset"] = next(offset, 12, items.len());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn course_scoping_search_pagination_and_read_only() {
        let root = std::env::temp_dir().join(format!("cruise-context-{}", id()));
        let s = Store::new(root.clone());
        let c =
            crate::core::call(&s, "create_course", json!({"code":"BIO","name":"Biology"})).unwrap();
        let other = crate::core::call(
            &s,
            "create_course",
            json!({"code":"CHEM","name":"Chemistry"}),
        )
        .unwrap();
        let cid = c["id"].as_str().unwrap();
        let l=crate::core::call(&s,"lecture.import",json!({"courseId":cid,"title":"Evolution","date":"2026-09-27","transcript":"WEBVTT\n\n00:00:01.000 --> 00:00:15.000\nSelection changes populations across generations."})).unwrap();
        let note=crate::core::call(&s,"save_course_source",json!({"courseId":cid,"title":"Selection notes","text":"Selection acts on heritable variation within populations."})).unwrap();
        let before = fs::read(s.path("state.json").unwrap()).unwrap();
        let expected =
            crate::core::rank_evidence(crate::core::all_evidence(&s, cid).unwrap(), "selection", 8);
        let result = read(&s, json!({"courseId":cid,"query":"selection"})).unwrap();
        assert_eq!(
            result["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["id"].clone())
                .collect::<Vec<_>>(),
            expected.iter().map(|s| s["id"].clone()).collect::<Vec<_>>()
        );
        for bad in [
            json!({"courseId":other["id"],"lectureId":l["id"]}),
            json!({"courseId":other["id"],"sourceId":format!("{}:t0",text(&l,"id"))}),
            json!({"courseId":cid,"sourceId":"../../.env.local"}),
            json!({"courseId":cid,"path":".env.local"}),
            json!({"query":"selection"}),
            json!({"courseId":cid,"lectureId":l["id"],"query":"selection"}),
            json!({"courseId":cid,"offset":-1}),
        ] {
            assert!(read(&s, bad).is_err());
        }
        assert_eq!(before, fs::read(s.path("state.json").unwrap()).unwrap());
        let full = format!(
            "# Long memory\n\n{}",
            "Selection and inheritance in populations. ".repeat(1100)
        );
        s.write(note["path"].as_str().unwrap(), full.as_bytes())
            .unwrap();
        let source = crate::core::rank_evidence(
            crate::core::all_evidence(&s, cid).unwrap(),
            "inheritance",
            8,
        )[0]["id"]
            .clone();
        let mut offset = 0;
        let mut joined = String::new();
        loop {
            let r = read(
                &s,
                json!({"courseId":cid,"sourceId":source,"offset":offset}),
            )
            .unwrap();
            joined.push_str(r["documentText"].as_str().unwrap());
            if r["nextOffset"].is_null() {
                break;
            }
            offset = r["nextOffset"].as_u64().unwrap();
        }
        assert_eq!(joined, full);
        fs::remove_dir_all(root).unwrap();
    }
}
