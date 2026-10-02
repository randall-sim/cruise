use super::*;

const COLORS: [(&str, &str, &str); 7] = [
    ("ink", "#172f2b", "#f3f6f5"),
    ("muted", "#53655d", "#f2f4f1"),
    ("green", "#315e42", "#e7f2e9"),
    ("blue", "#285c83", "#e9f3fa"),
    ("orange", "#874813", "#fff0dd"),
    ("purple", "#674784", "#f1eafa"),
    ("white", "#53655d", "#ffffff"),
];
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn coordinate(e: &Value, key: &str) -> f64 {
    e[key].as_f64().unwrap_or(0.0)
}
fn diagram_schema() -> Value {
    crate::mcp::catalog()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| text(t, "name") == "save_capture_artifact")
        .unwrap()["inputSchema"]["properties"]["diagram"]
        .clone()
}
pub fn render_diagram(diagram: Value, title: &str) -> Result<(String, Value)> {
    let diagram = crate::mcp::validate_schema(&diagram_schema(), diagram)?;
    let width = coordinate(&diagram, "width");
    let height = coordinate(&diagram, "height");
    let mut shapes = vec![];
    for e in list(&diagram, "elements") {
        let x = coordinate(e, "x");
        let y = coordinate(e, "y");
        let size = coordinate(e, "size");
        let tone = text(e, "tone");
        let (_, stroke, fill) = COLORS
            .iter()
            .find(|c| c.0 == tone)
            .ok_or("Invalid diagram tone")?;
        let kind = text(e, "type");
        let (right, bottom) = match kind {
            "rect" => (x + coordinate(e, "width"), y + coordinate(e, "height")),
            "ellipse" => (
                coordinate(e, "cx") + coordinate(e, "rx"),
                coordinate(e, "cy") + coordinate(e, "ry"),
            ),
            "line" => (
                coordinate(e, "x1").max(coordinate(e, "x2")),
                coordinate(e, "y1").max(coordinate(e, "y2")),
            ),
            _ => (
                x,
                y + (list(e, "lines").len().saturating_sub(1) as f64) * size * 1.4,
            ),
        };
        if right > width
            || bottom > height
            || (kind == "ellipse"
                && (coordinate(e, "cx") < coordinate(e, "rx")
                    || coordinate(e, "cy") < coordinate(e, "ry")))
        {
            return Err("Element extends beyond the diagram canvas".into());
        }
        let shape = match kind {
            "rect" => format!(
                "<rect x=\"{x}\" y=\"{y}\" width=\"{}\" height=\"{}\" rx=\"14\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"2\"/>",
                coordinate(e, "width"),
                coordinate(e, "height")
            ),
            "ellipse" => format!(
                "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"2\"/>",
                coordinate(e, "cx"),
                coordinate(e, "cy"),
                coordinate(e, "rx"),
                coordinate(e, "ry")
            ),
            "line" => format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{stroke}\" stroke-width=\"3\"{}{} />",
                coordinate(e, "x1"),
                coordinate(e, "y1"),
                coordinate(e, "x2"),
                coordinate(e, "y2"),
                if e["dashed"] == true {
                    " stroke-dasharray=\"9 7\""
                } else {
                    ""
                },
                if e["arrow"] == true {
                    format!(" marker-end=\"url(#arrow-{tone})\"")
                } else {
                    String::new()
                }
            ),
            _ => format!(
                "<text fill=\"{stroke}\" font-family=\"{}\" font-size=\"{size}\" font-weight=\"{}\" text-anchor=\"{}\">{}</text>",
                if e["mono"] == true {
                    "DejaVu Sans Mono, monospace"
                } else {
                    "DejaVu Sans, sans-serif"
                },
                if e["bold"] == true { 700 } else { 400 },
                text(e, "anchor"),
                strings(e, "lines")
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!(
                        "<tspan x=\"{x}\" y=\"{}\">{}</tspan>",
                        y + i as f64 * size * 1.4,
                        escape(line)
                    ))
                    .collect::<Vec<_>>()
                    .join("")
            ),
        };
        shapes.push(shape);
    }
    let markers=COLORS.iter().map(|(tone,color,_)|format!("<marker id=\"arrow-{tone}\" markerWidth=\"10\" markerHeight=\"10\" refX=\"8\" refY=\"3\" orient=\"auto\" markerUnits=\"strokeWidth\"><path d=\"M0,0 L0,6 L8,3 z\" fill=\"{color}\"/></marker>")).collect::<Vec<_>>().join("");
    Ok((
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" role=\"img\"><title>{} — reconstruction from course evidence</title><defs>{markers}</defs><rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>{}</svg>",
            escape(title),
            shapes.join("\n")
        ),
        diagram,
    ))
}

pub fn save(store: &Store, args: Value) -> Result<Value> {
    let args = crate::mcp::validate_input("save_capture_artifact", args)?;
    let has_diagram = args.get("diagram").is_some();
    let has_image = args.get("image").is_some();
    if has_diagram == has_image {
        return Err("Supply exactly one diagram or image".into());
    }
    let (bytes, definition) = if has_diagram {
        let (svg, diagram) = render_diagram(args["diagram"].clone(), text(&args, "title"))?;
        (svg.into_bytes(), Some(diagram))
    } else {
        (
            crate::core::normalize_image(arg(&args, "image")?, 2400, 20_000_000)?,
            None,
        )
    };
    store.mutate(|state| {
        let li=index(state,"lectures",arg(&args,"lectureId")?).map_err(|_|"Capture not found in this lecture")?;
        let l=state["lectures"][li].clone();
        let ci=list(&l,"captures").iter().position(|c|c["id"]==args["captureId"]).ok_or("Capture not found in this lecture")?;
        let capture=&l["captures"][ci];
        if list(capture,"artifacts").len()>=10{return Err("At most ten reconstructions per capture".into());}
        let available=evidence(store,state,text(&l,"courseId"))?;
        let mut sources=vec![];
        for id in unique(list(&args,"sourceIds").to_vec()) {
            let source=available.iter().find(|s|s["id"]==id && text(s,"kind")!="assignment").ok_or_else(||format!("Invalid reconstruction source: {}",id.as_str().unwrap_or("")))?;
            sources.push(source.clone());
        }
        if !sources.iter().any(|s|["transcript","note"].contains(&text(s,"kind"))){return Err("A reconstruction needs original transcript or course-note support".into());}
        let artifact_id=id();let base=format!("courses/{}/lectures/{}/reconstructions/{}/{artifact_id}",text(&l,"courseId"),text(&l,"id"),text(capture,"id"));
        let mut artifact=json!({"id":artifact_id,"title":text(&args,"title").trim(),"description":text(&args,"description").trim(),"createdAt":now(),"format":if has_diagram{"diagram"}else{"image"},"file":format!("{base}.{}",if has_diagram{"svg"}else{"png"}),"sourceIds":sources.iter().map(|s|s["id"].clone()).collect::<Vec<_>>(),"uncertainties":args["uncertainties"]});
        if has_diagram {artifact["definitionPath"]=json!(format!("{base}.json"));}
        store.write(text(&artifact,"file"),&bytes)?;
        if let Some(definition)=definition {write_json(store,text(&artifact,"definitionPath"),&definition)?;}
        let mut provenance=artifact.clone();provenance["lectureId"]=l["id"].clone();provenance["captureId"]=capture["id"].clone();provenance["originalPath"]=capture["file"].clone();provenance["seconds"]=capture["seconds"].clone();provenance["sources"]=json!(sources);provenance["status"]=json!("Generated interpretation; not an original lecture capture or recovered transcription");
        write_json(store,&format!("{base}.provenance.json"),&provenance)?;
        items_mut(&mut state["lectures"][li]["captures"][ci],"artifacts").push(artifact.clone());Ok(artifact)
    })
}

pub fn review(store: &Store, args: Value) -> Result<Value> {
    let args = crate::mcp::validate_input("review_capture_readability", args)?;
    store.mutate(|state| {
        let li=index(state,"lectures",arg(&args,"lectureId")?)?;
        let reviews=list(&args,"reviews");
        if reviews.iter().map(|r|text(r,"captureId")).collect::<HashSet<_>>().len()!=reviews.len(){return Err("Duplicate capture reviews".into());}
        let mut indices=vec![];
        for r in reviews {indices.push(list(&state["lectures"][li],"captures").iter().position(|c|c["id"]==r["captureId"]).ok_or("Reviewed capture not found in this lecture")?);}
        let reviewed_at=now();
        for (r,ci) in reviews.iter().zip(indices){state["lectures"][li]["captures"][ci]["readability"]=json!({"status":r["status"],"reason":text(r,"reason").trim(),"reviewedAt":reviewed_at});}
        let l=&state["lectures"][li];
        write_json(store,&format!("courses/{}/lectures/{}/capture-readability.json",text(l,"courseId"),text(l,"id")),&json!(list(l,"captures").iter().map(|c|json!({"captureId":c["id"],"review":c["readability"]})).collect::<Vec<_>>()))?;
        Ok(json!({"lectureId":l["id"],"reviewed":reviews.len()}))
    })
}
