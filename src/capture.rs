//! User-started HTML5 lecture sampler. Connects to an existing loopback CDP tab;
//! never launches a browser, navigates, exports credentials, or closes Chrome.
use crate::store::{Result, Store, arg, now, require_course};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::imageops::FilterType;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};
use tungstenite::{Message, WebSocket};
use url::Url;

const HELP: &str = "cruise capture --url <exact-open-tab-url> --list\ncruise capture --url <exact-open-tab-url> --course <id> --title <name> [--date YYYY-MM-DD] [--transcript file.vtt] [--streams 0,1] [--interval 3] [--kind slide|whiteboard|demo]\n\nConnects to existing Chrome via CHROME_CDP_URL (default http://127.0.0.1:9222). Captures are candidates requiring visual review; this does not create a lecture guide.";

#[derive(Debug)]
struct Options {
    url: String,
    course: Option<String>,
    title: Option<String>,
    date: String,
    transcript: Option<String>,
    streams: Option<String>,
    interval: f64,
    kind: String,
    list: bool,
}
impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        let mut values = json!({});
        let mut i = 0;
        while i < args.len() {
            let (key, inline) = args[i]
                .split_once('=')
                .map_or((args[i].as_str(), None), |(k, v)| (k, Some(v)));
            if key == "--list" && inline.is_none() {
                values["list"] = json!(true);
                i += 1;
                continue;
            }
            if !matches!(
                key,
                "--url"
                    | "--course"
                    | "--title"
                    | "--date"
                    | "--transcript"
                    | "--streams"
                    | "--interval"
                    | "--kind"
            ) {
                return Err(format!(
                    "Unknown capture option: {key}. Run cruise capture --help."
                ));
            }
            let value = if let Some(value) = inline {
                value.to_string()
            } else {
                i += 1;
                args.get(i)
                    .filter(|v| !v.starts_with("--"))
                    .cloned()
                    .ok_or_else(|| format!("Missing value for {key}"))?
            };
            values[key.trim_start_matches("--")] = json!(value);
            i += 1;
        }
        let url = values["url"].as_str().filter(|v|!v.is_empty()).ok_or("Pass --url with the exact URL of your open lecture tab. Use --list first to inspect streams.")?.to_string();
        let interval = values["interval"]
            .as_str()
            .unwrap_or("3")
            .parse::<f64>()
            .map_err(|_| "--interval must be between 1 and 30 seconds")?;
        if !interval.is_finite() || !(1.0..=30.0).contains(&interval) {
            return Err("--interval must be between 1 and 30 seconds".into());
        }
        let kind = values["kind"].as_str().unwrap_or("slide").to_string();
        if !matches!(kind.as_str(), "slide" | "whiteboard" | "demo") {
            return Err("--kind must be slide, whiteboard or demo".into());
        }
        Ok(Self {
            url,
            interval,
            kind,
            course: values["course"].as_str().map(str::to_string),
            title: values["title"].as_str().map(str::to_string),
            date: values["date"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| now()[..10].to_string()),
            transcript: values["transcript"].as_str().map(str::to_string),
            streams: values["streams"].as_str().map(str::to_string),
            list: values["list"] == true,
        })
    }
    fn indices(&self, count: usize) -> Result<Vec<usize>> {
        let indices = if let Some(streams) = &self.streams {
            streams
                .split(',')
                .map(|s| {
                    s.trim()
                        .parse::<usize>()
                        .map_err(|_| "Invalid stream indices".to_string())
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            (0..count).collect()
        };
        if indices.is_empty()
            || indices.iter().any(|i| *i >= count)
            || indices.iter().collect::<BTreeSet<_>>().len() != indices.len()
        {
            return Err("Invalid stream indices".into());
        }
        Ok(indices)
    }
}

fn endpoint(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|e| format!("Invalid Chrome CDP endpoint: {e}"))?;
    if !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        || !matches!(url.scheme(), "http" | "ws")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Chrome CDP must use an HTTP or WebSocket loopback endpoint without credentials".into(),
        );
    }
    Ok(url)
}
fn tcp(url: &Url) -> Result<TcpStream> {
    let host = url
        .host_str()
        .ok_or("CDP endpoint has no host")?
        .trim_matches(['[', ']']);
    let port = url
        .port_or_known_default()
        .ok_or("CDP endpoint has no port")?;
    let addresses = (host, port).to_socket_addrs().map_err(|e| e.to_string())?;
    let mut error = "No loopback address resolved".to_string();
    for address in addresses {
        if !address.ip().is_loopback() {
            return Err("Chrome CDP resolved outside loopback".into());
        }
        match TcpStream::connect_timeout(&address, Duration::from_secs(5)) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(20)))
                    .map_err(|e| e.to_string())?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(20)))
                    .map_err(|e| e.to_string())?;
                return Ok(stream);
            }
            Err(e) => error = e.to_string(),
        }
    }
    Err(format!(
        "Could not connect to the existing Chrome CDP endpoint: {error}"
    ))
}
fn discover(url: &Url) -> Result<String> {
    if url.scheme() == "ws" {
        return Ok(url.to_string());
    }
    let mut stream = tcp(url)?;
    let path = format!("{}/json/version", url.path().trim_end_matches('/'));
    let host = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap()),
        None => url.host_str().unwrap().into(),
    };
    write!(stream,"GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json\r\n\r\n").map_err(|e|e.to_string())?;
    let mut bytes = vec![];
    loop {
        let mut buffer = [0u8; 8192];
        let read = stream
            .read(&mut buffer)
            .map_err(|e| format!("Chrome CDP discovery: {e}"))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > 2_000_000 {
            return Err("Chrome CDP discovery response is too large".into());
        }
        if let Some(split) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..split]).map_err(|e| e.to_string())?;
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim())
                })
                .map(|v| v.parse::<usize>().map_err(|e| e.to_string()))
                .transpose()?;
            if content_length.is_some_and(|size| bytes.len() - split - 4 >= size) {
                break;
            }
            if headers.lines().any(|l| {
                l.to_ascii_lowercase().starts_with("transfer-encoding:")
                    && l.to_ascii_lowercase().contains("chunked")
            }) && decode_chunked(&bytes[split + 4..])?.is_some()
            {
                break;
            }
        }
    }
    let split = bytes
        .windows(4)
        .position(|v| v == b"\r\n\r\n")
        .ok_or("Invalid Chrome CDP HTTP response")?;
    let headers = std::str::from_utf8(&bytes[..split]).map_err(|e| e.to_string())?;
    if !headers
        .lines()
        .next()
        .is_some_and(|l| l.starts_with("HTTP/1.1 200 ") || l.starts_with("HTTP/1.0 200 "))
    {
        return Err("Chrome CDP discovery failed; expected HTTP 200 from /json/version".into());
    }
    let body = &bytes[split + 4..];
    let decoded;
    let body = if headers.lines().any(|l| {
        l.to_ascii_lowercase().starts_with("transfer-encoding:")
            && l.to_ascii_lowercase().contains("chunked")
    }) {
        decoded = decode_chunked(body)?.ok_or("Truncated CDP response")?;
        decoded.as_slice()
    } else {
        body
    };
    let value: Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let websocket = arg(&value, "webSocketDebuggerUrl")?.to_string();
    endpoint(&websocket)?;
    Ok(websocket)
}

fn decode_chunked(mut input: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut output = vec![];
    loop {
        let Some(end) = input.windows(2).position(|v| v == b"\r\n") else {
            return Ok(None);
        };
        let size = usize::from_str_radix(
            std::str::from_utf8(&input[..end])
                .map_err(|e| e.to_string())?
                .split(';')
                .next()
                .unwrap_or(""),
            16,
        )
        .map_err(|e| e.to_string())?;
        input = &input[end + 2..];
        if size == 0 {
            return Ok(Some(output));
        }
        if size > 2_000_000 {
            return Err("CDP response chunk is too large".into());
        }
        if input.len() < size + 2 {
            return Ok(None);
        }
        if &input[size..size + 2] != b"\r\n" {
            return Err("Invalid CDP response chunk".into());
        }
        output.extend_from_slice(&input[..size]);
        input = &input[size + 2..];
    }
}

#[derive(Clone, Debug)]
struct Session {
    id: String,
    parent: Option<String>,
    root_frame: String,
}
struct Cdp {
    socket: WebSocket<TcpStream>,
    next: u64,
    sessions: Vec<Session>,
}
impl Cdp {
    fn connect(url: &str) -> Result<Self> {
        let url = endpoint(url)?;
        let websocket = endpoint(&discover(&url)?)?;
        let stream = tcp(&websocket)?;
        let (socket, _) =
            tungstenite::client(websocket.as_str(), stream).map_err(|e| e.to_string())?;
        Ok(Self {
            socket,
            next: 0,
            sessions: vec![],
        })
    }
    fn call(&mut self, session: Option<&str>, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        let mut message = json!({"id":id,"method":method,"params":params});
        if let Some(session) = session {
            message["sessionId"] = json!(session)
        }
        self.socket
            .send(Message::Text(message.to_string().into()))
            .map_err(|e| format!("CDP {method}: {e}"))?;
        loop {
            let response = match self
                .socket
                .read()
                .map_err(|e| format!("CDP {method}: {e}"))?
            {
                Message::Text(text) => {
                    serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())?
                }
                Message::Close(_) => return Err("Chrome disconnected during capture".into()),
                _ => continue,
            };
            if response["method"] == "Target.attachedToTarget"
                && response["params"]["targetInfo"]["type"] == "iframe"
            {
                let sid = arg(&response["params"], "sessionId")?.to_string();
                if !self.sessions.iter().any(|s| s.id == sid) {
                    self.sessions.push(Session {
                        id: sid,
                        parent: response["sessionId"].as_str().map(str::to_string),
                        root_frame: String::new(),
                    });
                }
            }
            if response["id"].as_u64() != Some(id) {
                continue;
            }
            if let Some(error) = response.get("error") {
                return Err(format!(
                    "CDP {method}: {}",
                    error["message"]
                        .as_str()
                        .unwrap_or("Unknown protocol error")
                ));
            }
            return Ok(response["result"].clone());
        }
    }
    fn eval(
        &mut self,
        session: &str,
        context: i64,
        expression: &str,
        by_value: bool,
    ) -> Result<Value> {
        let response=self.call(Some(session),"Runtime.evaluate",json!({"expression":expression,"contextId":context,"returnByValue":by_value,"awaitPromise":true}))?;
        checked_runtime(response, by_value)
    }
    fn video(&mut self, video: &Video, function: &str, args: Vec<Value>) -> Result<Value> {
        let response=self.call(Some(&video.session),"Runtime.callFunctionOn",json!({"objectId":video.object,"functionDeclaration":function,"arguments":args.into_iter().map(|v|json!({"value":v})).collect::<Vec<_>>(),"returnByValue":true,"awaitPromise":true}))?;
        checked_runtime(response, true)
    }
    fn attach(&mut self, url: &str) -> Result<String> {
        let targets = self.call(None, "Target.getTargets", json!({}))?;
        let target=targets["targetInfos"].as_array().and_then(|t|t.iter().find(|t|t["type"]=="page"&&t["url"]==url)).ok_or("No open tab matches --url exactly. Sign in and open the lecture in the dedicated Chrome profile first.")?;
        let result = self.call(
            None,
            "Target.attachToTarget",
            json!({"targetId":target["targetId"],"flatten":true}),
        )?;
        let id = arg(&result, "sessionId")?.to_string();
        self.sessions.push(Session {
            id: id.clone(),
            parent: None,
            root_frame: String::new(),
        });
        Ok(id)
    }
    fn close(&mut self, root: &str) {
        let _ = self.call(None, "Target.detachFromTarget", json!({"sessionId":root}));
        let _ = self.socket.close(None);
    }
}
fn checked_runtime(response: Value, by_value: bool) -> Result<Value> {
    if !response["exceptionDetails"].is_null() {
        let exception = &response["exceptionDetails"];
        return Err(exception["exception"]["description"]
            .as_str()
            .or(exception["text"].as_str())
            .unwrap_or("Page evaluation failed")
            .to_string());
    }
    Ok(if by_value {
        response["result"]["value"].clone()
    } else {
        response["result"].clone()
    })
}
#[derive(Clone)]
struct World {
    session: String,
    context: i64,
    scroll: Value,
}
struct Video {
    session: String,
    object: String,
    frame_url: String,
    metadata: Value,
}
fn frame_nodes(tree: &Value, out: &mut Vec<(String, String)>) {
    if let (Some(id), Some(url)) = (tree["frame"]["id"].as_str(), tree["frame"]["url"].as_str()) {
        out.push((id.into(), url.into()));
    }
    if let Some(children) = tree["childFrames"].as_array() {
        for child in children {
            frame_nodes(child, out)
        }
    }
}
fn videos(cdp: &mut Cdp) -> Result<(Vec<Video>, Vec<World>)> {
    let mut videos = vec![];
    let mut worlds = vec![];
    let mut seen = BTreeSet::new();
    let mut index = 0;
    while index < cdp.sessions.len() {
        let session = cdp.sessions[index].id.clone();
        cdp.call(Some(&session), "Page.enable", json!({}))?;
        cdp.call(Some(&session),"Target.setAutoAttach",json!({"autoAttach":true,"waitForDebuggerOnStart":false,"flatten":true,"filter":[{"type":"iframe","exclude":false}]}))?;
        let tree = cdp.call(Some(&session), "Page.getFrameTree", json!({}))?;
        cdp.sessions[index].root_frame = arg(&tree["frameTree"]["frame"], "id")?.to_string();
        let mut frames = vec![];
        frame_nodes(&tree["frameTree"], &mut frames);
        for (frame, frame_url) in frames {
            if seen.contains(&frame) {
                continue;
            }
            // Out-of-process frames are visited through their own attached session.
            let created = match cdp.call(
                Some(&session),
                "Page.createIsolatedWorld",
                json!({"frameId":frame,"worldName":"cruise-capture"}),
            ) {
                Ok(v) => v,
                Err(e) if e.contains("No frame") || e.contains("not found") => continue,
                Err(e) => return Err(e),
            };
            let context = created["executionContextId"]
                .as_i64()
                .ok_or("Missing video execution context")?;
            seen.insert(frame);
            let metadata=cdp.eval(&session,context,"Array.from(document.querySelectorAll('video'), v => ({duration:v.duration,time:v.currentTime,paused:v.paused,muted:v.muted,trackModes:Array.from(v.textTracks,t=>t.mode),label:v.getAttribute('aria-label')||v.title||'HTML5 video'}))",true)?;
            let scroll = cdp.eval(&session, context, "({x:scrollX,y:scrollY})", true)?;
            worlds.push(World {
                session: session.clone(),
                context,
                scroll,
            });
            for (i, metadata) in metadata
                .as_array()
                .ok_or("Invalid HTML5 stream metadata")?
                .iter()
                .enumerate()
            {
                let object = cdp.eval(
                    &session,
                    context,
                    &format!("document.querySelectorAll('video')[{i}]"),
                    false,
                )?;
                videos.push(Video {
                    session: session.clone(),
                    object: arg(&object, "objectId")?.to_string(),
                    frame_url: frame_url.clone(),
                    metadata: metadata.clone(),
                });
            }
        }
        index += 1;
    }
    Ok((videos, worlds))
}

const SEEK: &str = r#"async function(t) {
  if (!this.isConnected) throw new Error('The lecture video was removed or the page changed');
  if (Math.abs(this.currentTime-t)<0.05 && this.readyState>=2) return;
  await new Promise((resolve,reject)=>{
    const video=this;
    const done=()=>{clearTimeout(timer);video.removeEventListener('seeked',done);resolve();};
    const timer=setTimeout(()=>{video.removeEventListener('seeked',done);reject(new Error('Video seek timed out'));},15000);
    video.addEventListener('seeked',done);video.currentTime=t;
  });
  if (Math.abs(this.currentTime-t)>0.1 || this.readyState<2)
    throw new Error(`Video could not seek to ${t}s (landed at ${this.currentTime}s); no capture saved for this timestamp`);
}"#;
const RESTORE: &str = r#"async function(state) {
  if (!this.isConnected) return;
  this.pause(); this.muted=state.muted;
  Array.from(this.textTracks).forEach((track,i)=>{track.mode=state.trackModes[i]||'disabled';});
  if (Math.abs(this.currentTime-state.time)>0.001) {
    await new Promise(resolve=>{const done=()=>{clearTimeout(timer);this.removeEventListener('seeked',done);resolve();};const timer=setTimeout(done,3000);this.addEventListener('seeked',done);this.currentTime=state.time;});
  }
  if (!state.paused) await this.play().catch(()=>{});
}"#;
fn screenshot(cdp: &mut Cdp, root: &str, video: &Video) -> Result<Vec<u8>> {
    cdp.video(video,"async function(){this.scrollIntoView({block:'center',inline:'center'});await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));}",vec![])?;
    let mut model = cdp.call(
        Some(&video.session),
        "DOM.getBoxModel",
        json!({"objectId":video.object}),
    )?;
    let quad = model["model"]["border"]
        .as_array()
        .ok_or("Video has no visible screenshot region")?;
    let coords = quad
        .iter()
        .map(|v| {
            v.as_f64()
                .ok_or_else(|| "Invalid video rectangle".to_string())
        })
        .collect::<Result<Vec<_>>>()?;
    if coords.len() != 8 {
        return Err("Invalid video rectangle".into());
    }
    let mut x = coords
        .iter()
        .step_by(2)
        .copied()
        .fold(f64::INFINITY, f64::min);
    let mut y = coords
        .iter()
        .skip(1)
        .step_by(2)
        .copied()
        .fold(f64::INFINITY, f64::min);
    let width = coords
        .iter()
        .step_by(2)
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        - x;
    let height = coords
        .iter()
        .skip(1)
        .step_by(2)
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        - y;
    let mut session = video.session.clone();
    while let Some(child) = cdp
        .sessions
        .iter()
        .find(|s| s.id == session && s.parent.is_some())
        .cloned()
    {
        let parent = child.parent.unwrap();
        let owner = cdp.call(
            Some(&parent),
            "DOM.getFrameOwner",
            json!({"frameId":child.root_frame}),
        )?;
        model = cdp.call(
            Some(&parent),
            "DOM.getBoxModel",
            json!({"backendNodeId":owner["backendNodeId"]}),
        )?;
        x += model["model"]["content"][0]
            .as_f64()
            .ok_or("Invalid frame position")?;
        y += model["model"]["content"][1]
            .as_f64()
            .ok_or("Invalid frame position")?;
        session = parent;
    }
    let metrics = cdp.call(Some(root), "Page.getLayoutMetrics", json!({}))?;
    let viewport = if metrics["cssLayoutViewport"].is_object() {
        &metrics["cssLayoutViewport"]
    } else {
        &metrics["layoutViewport"]
    };
    x += viewport["pageX"].as_f64().unwrap_or(0.);
    y += viewport["pageY"].as_f64().unwrap_or(0.);
    if ![x, y, width, height].iter().all(|v| v.is_finite()) || width <= 0. || height <= 0. {
        return Err(
            "Video is hidden or has no screenshot dimensions; expand the stream before capture"
                .into(),
        );
    }
    let result=cdp.call(Some(root),"Page.captureScreenshot",json!({"format":"png","captureBeyondViewport":true,"fromSurface":true,"clip":{"x":x,"y":y,"width":width,"height":height,"scale":1}}))?;
    STANDARD
        .decode(arg(&result, "data")?)
        .map_err(|e| e.to_string())
}
fn pixels(png: &[u8]) -> Result<Vec<u8>> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(image
        .resize_exact(96, 54, FilterType::Lanczos3)
        .into_luma8()
        .into_raw())
}
fn difference(previous: Option<&[u8]>, pixels: &[u8]) -> f64 {
    previous
        .map(|p| {
            pixels
                .iter()
                .zip(p)
                .map(|(a, b)| (*a as f64 - *b as f64).abs())
                .sum::<f64>()
                / (pixels.len() as f64 * 255.)
        })
        .unwrap_or(1.)
}
fn vtt_time(seconds: f64) -> String {
    let milliseconds = (seconds.max(0.) * 1000.).round() as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        milliseconds / 3_600_000,
        milliseconds / 60_000 % 60,
        milliseconds / 1000 % 60,
        milliseconds % 1000
    )
}
fn transcript(
    cdp: &mut Cdp,
    videos: &[Video],
    indices: &[usize],
    options: &Options,
) -> Result<String> {
    if let Some(path) = &options.transcript {
        let transcript = fs::read_to_string(path)
            .map_err(|e| format!("Could not read transcript {path}: {e}"))?;
        if !transcript.is_empty() {
            return Ok(transcript);
        }
    }
    for &index in indices {
        let cues=cdp.video(&videos[index],r#"async function(){
          for(const track of Array.from(this.textTracks)) track.mode='hidden';
          const tracks=()=>Array.from(this.textTracks).filter(t=>t.kind==='captions'||t.kind==='subtitles');
          const until=Date.now()+2500;while(!tracks().some(t=>t.cues&&t.cues.length)&&Date.now()<until) await new Promise(r=>setTimeout(r,100));
          return tracks().flatMap(t=>Array.from(t.cues||[],c=>({start:c.startTime,end:c.endTime,text:c.text||''})));
        }"#,vec![])?;
        if let Some(cues) = cues.as_array().filter(|c| !c.is_empty()) {
            return Ok(format!(
                "WEBVTT\n\n{}",
                cues.iter()
                    .map(|c| format!(
                        "{} --> {}\n{}",
                        vtt_time(c["start"].as_f64().unwrap_or(0.)),
                        vtt_time(c["end"].as_f64().unwrap_or(0.)),
                        c["text"].as_str().unwrap_or("")
                    ))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            ));
        }
    }
    Err("No native caption track is accessible. Download a VTT/SRT using the player, then pass --transcript <path>. No lecture was created.".into())
}
fn capture(
    store: &Store,
    options: &Options,
    cdp: &mut Cdp,
    root: &str,
    videos: &[Video],
    indices: &[usize],
    lecture_id: &mut Option<String>,
) -> Result<Value> {
    let course = options
        .course
        .as_deref()
        .ok_or("Pass --course <id> and --title <lecture name>")?;
    let title = options
        .title
        .as_deref()
        .filter(|t| !t.is_empty())
        .ok_or("Pass --course <id> and --title <lecture name>")?;
    require_course(&store.read()?, course)?;
    if indices.iter().any(|i| {
        videos[*i].metadata["duration"]
            .as_f64()
            .is_none_or(|d| !d.is_finite() || d <= 0. || d > 14400.)
    }) {
        return Err("Streams must be seekable recordings shorter than four hours. Load the recording first.".into());
    }
    let text = transcript(cdp, videos, indices, options)?;
    let coverage = format!(
        "Sampled {} HTML5 stream(s) every {}s. Visual changes are candidate captures, not a guarantee that every slide or final drawing was captured. Hidden/provider-specific streams and brief events between samples require manual review.",
        indices.len(),
        options.interval
    );
    let lecture = crate::core::call(
        store,
        "import_lecture",
        json!({"courseId":course,"title":title,"date":options.date,"sourceUrl":options.url,"transcript":text,"captureCoverage":format!("Capture in progress. {coverage}")}),
    )?;
    let lid = arg(&lecture, "id")?.to_string();
    *lecture_id = Some(lid.clone());
    let duration = indices
        .iter()
        .map(|i| videos[*i].metadata["duration"].as_f64().unwrap())
        .fold(lecture["duration"].as_f64().unwrap_or(0.), f64::max);
    store.mutate(|state| {
        let lecture = state["lectures"]
            .as_array_mut()
            .and_then(|v| v.iter_mut().find(|l| l["id"] == lid))
            .ok_or("Lecture not found")?;
        lecture["duration"] = json!(duration);
        Ok(())
    })?;
    let manifest_path = format!("courses/{course}/lectures/{lid}/capture-manifest.json");
    let mut manifest = json!({"coverage":coverage,"interval":options.interval,"streams":indices,"captures":[],"complete":false});
    crate::files::write_json(store, &manifest_path, &manifest)?;
    for &index in indices {
        let video = &videos[index];
        cdp.video(video, "function(){this.pause();this.muted=true;}", vec![])?;
        let mut previous: Option<Vec<u8>> = None;
        let mut previous_png: Option<Vec<u8>> = None;
        let mut previous_time = 0.;
        let mut last_saved = -100.;
        let mut last_saved_time = -1.;
        let end = (video.metadata["duration"].as_f64().unwrap() - 0.1)
            .max(0.)
            .min(duration);
        let mut time = 0.;
        loop {
            cdp.video(video, SEEK, vec![json!(time)])?;
            let png = screenshot(cdp, root, video)?;
            let current = pixels(&png)?;
            let preserve_previous = previous_time > last_saved_time;
            let periodic = time - last_saved >= 60. || time == end;
            let mut save = |image: &[u8], seconds: f64, reason: &str| -> Result<()> {
                if seconds == last_saved_time {
                    return Ok(());
                }
                crate::core::call(
                    store,
                    "save_capture",
                    json!({"lectureId":lid,"seconds":seconds,"stream":format!("Stream {index} · {}",video.metadata["label"].as_str().unwrap_or("HTML5 video")),"kind":options.kind,"image":format!("data:image/png;base64,{}",STANDARD.encode(image)),"caption":format!("Candidate capture ({reason}); visual review required.")}),
                )?;
                manifest["captures"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"stream":index,"seconds":seconds,"reason":reason}));
                crate::files::write_json(store, &manifest_path, &manifest)?;
                last_saved_time = seconds;
                last_saved = seconds;
                Ok(())
            };
            if difference(previous.as_deref(), &current) > 0.045 {
                if let Some(previous_png) = &previous_png
                    && preserve_previous
                {
                    save(previous_png, previous_time, "before transition")?;
                }
                save(&png, time, "visual change")?;
            } else if periodic {
                save(&png, time, "periodic/final checkpoint")?;
            }
            previous = Some(current);
            previous_png = Some(png);
            previous_time = time;
            if time == end {
                break;
            }
            time = (time + options.interval).min(end);
        }
    }
    manifest["complete"] = json!(true);
    crate::files::write_json(store, &manifest_path, &manifest)?;
    crate::core::call(
        store,
        "set_lecture_coverage",
        json!({"lectureId":lid,"coverage":coverage}),
    )?;
    Ok(
        json!({"lectureId":lid,"captures":manifest["captures"].as_array().unwrap().len(),"manifestPath":manifest_path,"coverage":coverage}),
    )
}

pub fn run(store: &Store, args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return Ok(());
    }
    let options = Options::parse(args)?;
    let endpoint =
        std::env::var("CHROME_CDP_URL").unwrap_or_else(|_| "http://127.0.0.1:9222".into());
    let result = execute(store, &options, &endpoint)?;
    if !options.list {
        println!(
            "Saved {} candidate captures. Review lecture {} in cruise, then queue its guide.",
            result["captures"],
            arg(&result, "lectureId")?
        );
    }
    Ok(())
}
fn execute(store: &Store, options: &Options, endpoint: &str) -> Result<Value> {
    let mut cdp = Cdp::connect(endpoint)?;
    let root = cdp.attach(&options.url)?;
    let result = videos(&mut cdp);
    let (videos, worlds) = match result {
        Ok(v) => v,
        Err(e) => {
            cdp.close(&root);
            return Err(e);
        }
    };
    let listing:Vec<_>=videos.iter().enumerate().map(|(index,v)|json!({"index":index,"label":v.metadata["label"],"frame":v.frame_url,"duration":v.metadata["duration"]})).collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&listing).map_err(|e| e.to_string())?
    );
    let mut lecture_id = None;
    let mut touched = false;
    let result = (|| {
        if videos.is_empty() {
            return Err("No HTML5 video streams found. This player needs a provider adapter or a downloaded video/transcript workflow.".into());
        }
        if options.list {
            return Ok(json!(listing));
        }
        let indices = options.indices(videos.len())?;
        touched = true;
        capture(
            store,
            options,
            &mut cdp,
            &root,
            &videos,
            &indices,
            &mut lecture_id,
        )
    })();
    if let Err(error) = &result
        && let Some(lid) = &lecture_id
    {
        let coverage =
            format!("Partial capture: {error}. Review the saved images and retry missing streams.");
        let _ = crate::core::call(
            store,
            "set_lecture_coverage",
            json!({"lectureId":lid,"coverage":coverage}),
        );
    }
    if touched {
        for video in &videos {
            let _ = cdp.video(video, RESTORE, vec![video.metadata.clone()]);
        }
        for world in worlds.iter().rev() {
            let _ = cdp.eval(
                &world.session,
                world.context,
                &format!("scrollTo({},{})", world.scroll["x"], world.scroll["y"]),
                true,
            );
        }
    }
    cdp.close(&root);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_options_and_loopback_boundary() {
        let args = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let options = Options::parse(&args(&[
            "--url",
            "https://example.com/lecture",
            "--streams=1,0",
            "--interval",
            "1.5",
            "--kind",
            "whiteboard",
        ]))
        .unwrap();
        assert_eq!(options.indices(2).unwrap(), vec![1, 0]);
        assert_eq!(options.interval, 1.5);
        for input in [
            vec![],
            args(&["--url", "x", "--interval", "NaN"]),
            args(&["--url", "x", "--interval", "31"]),
            args(&["--url", "x", "--kind", "unknown"]),
        ] {
            assert!(Options::parse(&input).is_err())
        }
        let duplicate = Options::parse(&args(&["--url", "x", "--streams", "0,0"])).unwrap();
        assert!(duplicate.indices(2).is_err());
        for url in [
            "https://example.com:9222",
            "http://192.168.1.4:9222",
            "http://localhost.evil.test",
            "http://user:password@localhost:9222",
        ] {
            assert!(endpoint(url).is_err())
        }
        for url in [
            "http://127.0.0.1:9222",
            "http://localhost:9222",
            "http://[::1]:9222",
            "ws://127.0.0.1:9222/devtools/browser/abc",
        ] {
            assert!(endpoint(url).is_ok(), "{url}")
        }
    }
    #[test]
    fn image_change_threshold_and_vtt_timestamps() {
        assert_eq!(difference(None, &[0, 0]), 1.);
        assert_eq!(difference(Some(&[0, 0]), &[0, 0]), 0.);
        assert_eq!(difference(Some(&[0, 0]), &[255, 255]), 1.);
        assert_eq!(vtt_time(3661.234), "01:01:01.234");
    }

    #[cfg(unix)]
    struct BrowserFixture {
        child: std::process::Child,
        root: std::path::PathBuf,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        server: Option<std::thread::JoinHandle<()>>,
        url: String,
        endpoint: String,
    }
    #[cfg(unix)]
    impl BrowserFixture {
        fn launch() -> Option<Self> {
            use std::{
                os::unix::process::CommandExt,
                process::{Command, Stdio},
                sync::{
                    Arc,
                    atomic::{AtomicBool, Ordering},
                },
                thread,
                time::Instant,
            };
            let chrome = std::env::var("CRUISE_TEST_CHROME")
                .ok()
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    [
                        "/usr/bin/google-chrome",
                        "/usr/bin/chromium",
                        "/usr/bin/chromium-browser",
                    ]
                    .iter()
                    .map(std::path::PathBuf::from)
                    .find(|p| p.is_file())
                });
            let Some(chrome) = chrome else {
                eprintln!("Chrome fixture test skipped; set CRUISE_TEST_CHROME to run it.");
                return None;
            };
            let root =
                std::env::temp_dir().join(format!("cruise-capture-test-{}", crate::store::id()));
            fs::create_dir_all(root.join("chrome")).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let url = "https://course.example/lecture".to_string();
            let child = Command::new(chrome)
                .args([
                    "--headless=new",
                    "--no-sandbox",
                    "--disable-dev-shm-usage",
                    "--disable-gpu",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--remote-debugging-port=0",
                    "--window-size=900,700",
                    "--autoplay-policy=no-user-gesture-required",
                ])
                .arg(format!("--user-data-dir={}", root.join("chrome").display()))
                .arg("about:blank")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .unwrap();
            let mut fixture = Self {
                child,
                root,
                stop,
                server: None,
                url,
                endpoint: String::new(),
            };
            let start = Instant::now();
            let port = loop {
                if let Ok(text) = fs::read_to_string(fixture.root.join("chrome/DevToolsActivePort"))
                    && let Some(port) = text.lines().next()
                {
                    break port.to_string();
                }
                assert!(
                    start.elapsed() < Duration::from_secs(10),
                    "Test Chrome did not expose its debug port"
                );
                thread::sleep(Duration::from_millis(25));
            };
            fixture.endpoint = format!("http://127.0.0.1:{port}");
            // Fulfill a synthetic HTTPS page through this test-owned browser's
            // CDP session. No real network, account, browser profile, or TLS key.
            let mut router = Cdp::connect(&fixture.endpoint).unwrap();
            let session = router.attach("about:blank").unwrap();
            router
                .call(
                    Some(&session),
                    "Fetch.enable",
                    json!({"patterns":[{"urlPattern":"https://course.example/*"}]}),
                )
                .unwrap();
            router.next += 1;
            router.socket.send(Message::Text(json!({"id":router.next,"sessionId":session,"method":"Page.navigate","params":{"url":fixture.url}}).to_string().into())).unwrap();
            router
                .socket
                .get_mut()
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            fixture.server = Some(thread::spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    let message = match router.socket.read() {
                        Ok(Message::Text(text)) => serde_json::from_str::<Value>(&text).unwrap(),
                        Ok(Message::Close(_)) => break,
                        Ok(_) => continue,
                        Err(tungstenite::Error::Io(e))
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            continue;
                        }
                        Err(_) => break,
                    };
                    if message["method"] != "Fetch.requestPaused" {
                        continue;
                    }
                    let request = message["params"]["request"]["url"].as_str().unwrap_or("");
                    let (mut body, content_type): (&[u8], &str) = if request.ends_with(".webm") {
                        (
                            include_bytes!("../tests/fixtures/lecture.webm"),
                            "video/webm",
                        )
                    } else if request.ends_with(".vtt") {
                        (b"WEBVTT\n\n00:00:00.000 --> 00:00:04.000\nThe captions end before the final visual frame.\n","text/vtt")
                    } else {
                        (b"<!doctype html><meta charset=utf-8><style>body{margin:0}video{display:block;width:320px;height:180px}</style><video aria-label='Slides' preload='auto' src='/video.webm'><track kind='captions' src='/captions.vtt' default></video><video aria-label='Board' preload='auto' src='/video.webm'><track kind='captions' src='/captions.vtt' default></video>","text/html")
                    };
                    let mut headers = vec![json!({"name":"Content-Type","value":content_type})];
                    let mut status = 200;
                    if request.ends_with(".webm") {
                        headers.push(json!({"name":"Accept-Ranges","value":"bytes"}));
                        let range = message["params"]["request"]["headers"]
                            .as_object()
                            .and_then(|h| {
                                h.iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case("range"))
                                    .and_then(|(_, v)| v.as_str())
                            });
                        if let Some(range) = range
                            .and_then(|r| r.strip_prefix("bytes="))
                            .and_then(|r| r.split_once('-'))
                        {
                            let length = body.len();
                            let start = range.0.parse::<usize>().unwrap_or(0);
                            let end = range
                                .1
                                .parse::<usize>()
                                .unwrap_or(length - 1)
                                .min(length - 1);
                            body = &body[start..=end];
                            status = 206;
                            headers.push(json!({"name":"Content-Range","value":format!("bytes {start}-{end}/{length}")}));
                        }
                    }
                    router.next += 1;
                    let response = json!({"id":router.next,"sessionId":session,"method":"Fetch.fulfillRequest","params":{"requestId":message["params"]["requestId"],"responseCode":status,"responseHeaders":headers,"body":STANDARD.encode(body)}});
                    if router
                        .socket
                        .send(Message::Text(response.to_string().into()))
                        .is_err()
                    {
                        break;
                    }
                }
            }));
            let start = Instant::now();
            let mut readiness = Cdp::connect(&fixture.endpoint).unwrap();
            while readiness.attach(&fixture.url).is_err() {
                assert!(
                    start.elapsed() < Duration::from_secs(5),
                    "Fixture target did not navigate"
                );
                thread::sleep(Duration::from_millis(20));
            }
            Some(fixture)
        }
    }
    #[cfg(unix)]
    impl Drop for BrowserFixture {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = std::process::Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            let _ = self.child.wait();
            if let Some(server) = self.server.take() {
                let _ = server.join();
            }
            assert!(self.root.starts_with(std::env::temp_dir()));
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[cfg(unix)]
    #[test]
    fn isolated_chrome_sampling_preserves_final_frames_history_and_player_state() {
        let Some(mut fixture) = BrowserFixture::launch() else {
            return;
        };
        let store = Store {
            root: fixture.root.join("workspace"),
        };
        store
            .mutate(|v| {
                v["courses"] = json!([{"id":"course","code":"CAP","name":"Fixture"}]);
                Ok(())
            })
            .unwrap();
        let mut inspect = Cdp::connect(&fixture.endpoint).unwrap();
        let root = inspect.attach(&fixture.url).unwrap();
        let ready_start = std::time::Instant::now();
        loop {
            let ready=inspect.call(Some(&root),"Runtime.evaluate",json!({"expression":"document.readyState==='complete' && document.querySelectorAll('video').length===2","returnByValue":true}));
            if ready.is_ok_and(|v| v["result"]["value"] == true) {
                break;
            }
            assert!(
                ready_start.elapsed() < Duration::from_secs(8),
                "Fixture page did not load"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        let tree = inspect
            .call(Some(&root), "Page.getFrameTree", json!({}))
            .unwrap();
        let world=inspect.call(Some(&root),"Page.createIsolatedWorld",json!({"frameId":tree["frameTree"]["frame"]["id"],"worldName":"cruise-capture-test"})).unwrap()["executionContextId"].as_i64().unwrap();
        inspect.eval(&root,world,"new Promise((resolve,reject)=>{const started=Date.now();const check=()=>{const v=Array.from(document.querySelectorAll('video'));if(v.length===2&&v.every(v=>v.readyState>=2&&Number.isFinite(v.duration)))return resolve(true);if(Date.now()-started>5000)return reject(new Error('Fixture videos did not load'));setTimeout(check,25)};check()})",true).unwrap();
        let (streams, _) = videos(&mut inspect).unwrap();
        assert_eq!(streams.len(), 2);
        for video in &streams {
            inspect.video(video, SEEK, vec![json!(0.75)]).unwrap();
            inspect.video(video,"function(){this.pause();this.muted=false;for(const t of Array.from(this.textTracks))t.mode='showing';}",vec![]).unwrap();
        }
        let original=inspect.eval(&root,world,"Array.from(document.querySelectorAll('video'),v=>({time:v.currentTime,paused:v.paused,muted:v.muted,tracks:Array.from(v.textTracks,t=>t.mode)}))",true).unwrap();
        let options = Options {
            url: fixture.url.clone(),
            course: Some("course".into()),
            title: Some("Native video fixture".into()),
            date: "2026-10-02".into(),
            transcript: None,
            streams: None,
            interval: 1.,
            kind: "slide".into(),
            list: false,
        };
        let mut list =
            Options::parse(&["--url".into(), fixture.url.clone(), "--list".into()]).unwrap();
        let listing = execute(&store, &list, &fixture.endpoint).unwrap();
        assert_eq!(listing.as_array().unwrap().len(), 2);
        assert_eq!(store.read().unwrap()["lectures"], json!([]));
        list.url.push_str("missing");
        assert!(
            execute(&store, &list, &fixture.endpoint)
                .unwrap_err()
                .contains("matches --url exactly")
        );
        let result = execute(&store, &options, &fixture.endpoint).unwrap();
        let state = store.read().unwrap();
        let lecture = &state["lectures"][0];
        assert!(lecture["duration"].as_f64().unwrap() > 4.);
        assert!(lecture["captures"].as_array().unwrap().len() >= 6);
        let manifest =
            crate::files::read_json(&store, arg(&result, "manifestPath").unwrap(), Value::Null)
                .unwrap();
        assert_eq!(manifest["complete"], true);
        let captures = manifest["captures"].as_array().unwrap();
        assert!(captures.iter().any(|v| v["reason"] == "before transition"));
        for stream in 0..2 {
            assert!(
                captures
                    .iter()
                    .any(|v| v["stream"] == stream && v["seconds"].as_f64().unwrap() > 4.)
            );
        }
        assert_eq!(inspect.eval(&root,world,"Array.from(document.querySelectorAll('video'),v=>({time:v.currentTime,paused:v.paused,muted:v.muted,tracks:Array.from(v.textTracks,t=>t.mode)}))",true).unwrap(),original);
        inspect
            .eval(
                &root,
                world,
                "document.querySelectorAll('video')[1].style.display='none'",
                true,
            )
            .unwrap();
        assert!(execute(&store, &options, &fixture.endpoint).is_err());
        let state = store.read().unwrap();
        let partial = &state["lectures"][1];
        assert!(
            partial["captureCoverage"]
                .as_str()
                .unwrap()
                .starts_with("Partial capture:")
        );
        assert!(!partial["captures"].as_array().unwrap().is_empty());
        assert_eq!(inspect.eval(&root,world,"Array.from(document.querySelectorAll('video'),v=>({time:v.currentTime,paused:v.paused,muted:v.muted,tracks:Array.from(v.textTracks,t=>t.mode)}))",true).unwrap(),original);
        assert!(fixture.child.try_wait().unwrap().is_none());
        inspect.close(&root);
    }
}
