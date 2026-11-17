//! Studio as a setup for Spumcake-Skills: the skills it finds, and the
//! assistant's project tools.
//!
//! Every project is organized the way the Last order benchmark is, as the
//! owner approved on 2026-11-17, and in every one `request.py` is the one
//! renderer, as the Studio-as-a-setup contract says: every skill is listed
//! with a note saying so, and the tools here only read, look, and write
//! where the agent may write by hand. Every other folder is written only by
//! Skills' own scripts.

use protocol::{Tool, ToolGroup, ToolResult};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};

/// The host's tool group for the project.
pub(crate) const PROJECT_TOOLS: &str = "project";
const LIST_FILES: &str = "list_files";
const READ_FILE: &str = "read_file";
const LOOK: &str = "look_at_file";
const WRITE_FILE: &str = "write_file";
const READ_SKILL: &str = "read_skill";

/// What every skill carries in the project.
pub(crate) const RENDERS_HERE: &str = "renders through request.py here";

/// Studio's own folder in a project, which no step needs, and its name
/// before 0.15.
const OWN_FOLDERS: [&str; 2] = [".spumcake", ".inpainter"];

/// The most a read returns at once, so one file never floods the
/// conversation.
const MOST_TEXT: usize = 60_000;
/// The largest image looked at whole, below the byte limit one request's
/// images keep to.
const MOST_IMAGE: u64 = 8 * 1024 * 1024;
/// How deep a skill folder is searched for `SKILL.md`, enough for a
/// repository's `.project/skills/<category>/<name>/`.
const SKILL_DEPTH: usize = 5;

/// One skill a setup lists: what its front matter says, and where it is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Skill {
    pub name: String,
    pub description: String,
    pub implements: Vec<String>,
    pub helps: Vec<String>,
    /// The folder holding its `SKILL.md`.
    pub folder: PathBuf,
    /// The repository that holds it, which its relative links resolve in.
    pub root: PathBuf,
}

/// Every skill under the folders, in the folders' order, each name once:
/// the first folder that has a name wins.
pub(crate) fn find_skills(folders: &[PathBuf]) -> Vec<Skill> {
    let mut found: Vec<Skill> = Vec::new();
    for folder in folders {
        let mut here = Vec::new();
        walk(folder, SKILL_DEPTH, &mut here);
        here.sort();
        for path in here {
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Some(mut skill) = front_matter(&text) else { continue };
            if found.iter().any(|known| known.name == skill.name) {
                continue;
            }
            // A linked skill, as media-team links each one, belongs to the
            // repository its link points into.
            let dir = path.parent().unwrap_or(folder);
            let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
            skill.root = repository_root(&dir).unwrap_or_else(|| folder.clone());
            skill.folder = dir;
            found.push(skill);
        }
    }
    found
}

/// The skills under the folders, found again at most every few seconds,
/// so an edit to a skill reaches the assistant without a walk every frame.
pub(crate) fn listed_skills(folders: &[String]) -> Vec<Skill> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static FOUND: Mutex<Option<(Vec<String>, Instant, Vec<Skill>)>> = Mutex::new(None);
    let Ok(mut found) = FOUND.lock() else { return find_skills(&folders.iter().map(PathBuf::from).collect::<Vec<_>>()) };
    if let Some((same, at, skills)) = found.as_ref()
        && same == folders
        && at.elapsed() < Duration::from_secs(5)
    {
        return skills.clone();
    }
    let skills = find_skills(&folders.iter().map(PathBuf::from).collect::<Vec<_>>());
    *found = Some((folders.to_vec(), Instant::now(), skills.clone()));
    skills
}

fn walk(folder: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let file = folder.join("SKILL.md");
    if file.is_file() {
        out.push(file);
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // `.project` holds a repository's own skills; other hidden folders,
        // and builds, hold none.
        let skip = (name.starts_with('.') && name != ".project") || name == "target" || name == "node_modules";
        // `is_dir` follows a link, so a folder of linked skills is walked.
        if !skip && entry.path().is_dir() {
            walk(&entry.path(), depth - 1, out);
        }
    }
}

/// The nearest folder at or above `dir` that is a git checkout.
fn repository_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|folder| folder.join(".git").exists()).map(Path::to_path_buf)
}

/// A skill's front matter: `name`, `description`, and the lists
/// `implements` and `helps`, each on one line.
fn front_matter(text: &str) -> Option<Skill> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut skill = Skill {
        name: String::new(),
        description: String::new(),
        implements: Vec::new(),
        helps: Vec::new(),
        folder: PathBuf::new(),
        root: PathBuf::new(),
    };
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        let list = || {
            value
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|item| item.trim().trim_matches(['"', '\'']).to_string())
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
        };
        match key.trim() {
            "name" => skill.name = value.trim_matches(['"', '\'']).to_string(),
            "description" => skill.description = value.trim_matches(['"', '\'']).to_string(),
            "implements" => skill.implements = list(),
            "helps" => skill.helps = list(),
            _ => {}
        }
    }
    (!skill.name.is_empty()).then_some(skill)
}

/// The host's tool group for the project at `folder`, with the skills
/// listed in its instructions, each with the note that it renders through
/// `request.py` here.
pub(crate) fn tool_group(folder: &Path, skills: &[Skill]) -> ToolGroup {
    let mut instructions = format!(
        "This project, {}, is organized as Spumcake-Skills' docs/projects.md describes: its folders and file names say what it holds. \
In this project, request.py is the one renderer: a model is called only through it and the plan card, never through an execution skill or a command. \
Read a skill with read_skill before following it, and read what it links to with read_file, relative to the skill's folder or its repository. \
Read the project with list_files and read_file, and look at every image you judge with look_at_file. \
Write by hand only a page of brief/, or a new file in the agent's folder with its line in its index; every other folder is written by the skills' own scripts. \
Put a take into the chain only with publish_take, after reading what pipeline.py touches says it reaches and telling the user: it shows the publish card, and a commit happens only when the user leaves Commit on. Give it, in from, every canon artifact the take was made from that the skill says to pass with --from. \
Run a script or any other command with run_command, which the user approves first. Send a request to a model only with send_request, from a spec the skill built and you wrote into the agent's folder, or a pipeline's step with rerun_step: the user approves its plan and cost first. Tell the user, word for word, what request.py tell prints for each request. \
A prompt for a language model is sent the same way, as a request of its own with send_request, never answered by you, so its answer has a report. \
A request still at its endpoint has a pending record in requests/; finish it with run_command and request.py collect <id>, or cancel it with request.py cancel <id>, neither of which spends. \
Before a request or a script that can take minutes, say in one line what runs and roughly how long it will take; the user sees its output as it runs and can stop it. \
Research, such as a skill's references, is done here: ask for all the searches one research needs with one web_search, which the user approves with its cost; read a page with fetch_page, which is free; and download each file you would keep with download_file, which the user approves, naming its page and licence. Look at each download, then file what you keep into sources/ with send_request and a spec of request.py's research kind: its outputs are the downloaded files, and its sources give each one's address, page, and the use allowed.",
        folder.display()
    );
    // Requests still at their endpoint, which the user can have
    // collected or cancelled.
    let mut pending: Vec<String> = std::fs::read_dir(folder.join("requests"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().to_string_lossy().strip_suffix(".pending.json").map(String::from))
        .collect();
    pending.sort();
    if !pending.is_empty() {
        instructions.push_str(&format!(
            "\n\nPending at their endpoint now, already paid for: {}. Tell the user, and offer to collect or cancel each.",
            pending.join(", ")
        ));
    }
    if skills.is_empty() {
        instructions.push_str("\n\nNo skill folder is set, so no skill is listed. Say so when a skill is needed, and stop.");
    } else {
        instructions.push_str("\n\nThe skills, each of which ");
        instructions.push_str(RENDERS_HERE);
        instructions.push(':');
        for skill in skills {
            instructions.push_str(&format!("\n- {}: {} ({})", skill.name, skill.description.trim(), RENDERS_HERE));
        }
    }
    let path = |what: &str| json!({ "type": "string", "description": what });
    ToolGroup {
        id: PROJECT_TOOLS.into(),
        name: "Project".into(),
        description: "Read the project and the skills, look at its images, write by hand where the agent may, run Skills' scripts and requests on the user's yes, and research.".into(),
        instructions,
        tools: vec![
            Tool {
                name: LIST_FILES.into(),
                description: "List a folder of the project, or of a skill's repository: each entry's name, whether it is a folder, and its size.".into(),
                input: json!({
                    "type": "object",
                    "properties": { "path": path("The folder, relative to the project, or absolute inside a skill's repository. Leave it out for the project's own folder.") },
                    "additionalProperties": false
                }),
                implements: Vec::new(),
            },
            Tool {
                name: READ_FILE.into(),
                description: "Read a text file of the project, or of a skill's repository. A request's report gives its record as JSON. A long file is read in parts, from a line.".into(),
                input: json!({
                    "type": "object",
                    "properties": {
                        "path": path("The file, relative to the project, or absolute inside a skill's repository."),
                        "from_line": { "type": "integer", "description": "The first line to read, from 1. Leave it out to read from the start." }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
                implements: Vec::new(),
            },
            Tool {
                name: LOOK.into(),
                description: "Look at an image of the project, or of a skill's repository: PNG, JPEG, WebP, or GIF. The image is shown to you, and the user sees it with its full path.".into(),
                input: json!({
                    "type": "object",
                    "properties": { "path": path("The image, relative to the project, or absolute inside a skill's repository.") },
                    "required": ["path"],
                    "additionalProperties": false
                }),
                implements: Vec::new(),
            },
            Tool {
                name: WRITE_FILE.into(),
                description: "Write a page of brief/, or add a new file to the agent's folder, named by the local time and what it is, with its line in the folder's index. Never anything else.".into(),
                input: json!({
                    "type": "object",
                    "properties": {
                        "folder": { "type": "string", "enum": ["brief", "agent"], "description": "brief for a brief page, written in place; agent for a new file of the agent's own." },
                        "name": { "type": "string", "description": "For brief, the page's file name, such as 002-last-order.html. For agent, what the file is, with its extension, such as door-check.md." },
                        "text": { "type": "string", "description": "The whole file." },
                        "about": { "type": "string", "description": "For agent, one line saying what the file is, for the index." }
                    },
                    "required": ["folder", "name", "text"],
                    "additionalProperties": false
                }),
                implements: Vec::new(),
            },
            Tool {
                name: READ_SKILL.into(),
                description: "Read a skill, unchanged, with its folder and its repository, which its relative links resolve in.".into(),
                input: json!({
                    "type": "object",
                    "properties": { "name": { "type": "string", "description": "The skill's name, as listed." } },
                    "required": ["name"],
                    "additionalProperties": false
                }),
                implements: Vec::new(),
            },
        ]
        .into_iter()
        .chain(crate::runner::tools())
        .chain([Tool {
            name: PUBLISH_TAKE.into(),
            description: "Put a take into the chain: publish it as a canon artifact through the publish card, which shows the user the version it writes and what a commit takes along, and runs only on their yes. Read pipeline.py touches --artifact \"<artifact>\" --json first and tell the user what it reaches. A take made from inputs that are no longer canon is refused.".into(),
            input: json!({
                "type": "object",
                "properties": {
                    "take": { "type": "string", "description": "The take's file, relative to the project, such as library/<file>." },
                    "artifact": { "type": "string", "description": "The canon artifact it is published as, \"<element>: <role>\"." },
                    "with": { "type": "array", "items": { "type": "string" }, "description": "Takes of its inputs published together with it, each \"<element>: <role>=<request id>\"." },
                    "from": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": { "artifact": { "type": "string" }, "role": { "type": "string" } },
                            "required": ["artifact", "role"],
                            "additionalProperties": false
                        },
                        "description": "Every canon artifact it was made from, \"<element>: <role>\", with the role it played, as the skill says to pass with --from. Those its step and its record name are added."
                    }
                },
                "required": ["take", "artifact"],
                "additionalProperties": false
            }),
            implements: Vec::new(),
        }])
        .collect(),
    }
}

/// The assistant's publish of a take, which the window carries out through
/// the publish card.
pub(crate) const PUBLISH_TAKE: &str = "publish_take";

/// Whether `tool`, a full tool key, is one of the project tools.
pub(crate) fn is_project_tool(tool: &str) -> bool {
    protocol::split_tool_key(tool).is_some_and(|(_, group, _)| group == PROJECT_TOOLS)
}

/// Run a project tool. `tool` is its name within the group.
pub(crate) fn call(folder: &Path, skills: &[Skill], call: String, tool: &str, input: &Value) -> ToolResult {
    let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let answer = match tool {
        LIST_FILES => list_files(folder, skills, &text("path")),
        READ_FILE => read_file(folder, skills, &text("path"), input.get("from_line").and_then(Value::as_u64).unwrap_or(1)),
        LOOK => return look(folder, skills, call, &text("path")),
        WRITE_FILE => write_file(folder, &text("folder"), &text("name"), &text("text"), &text("about")),
        READ_SKILL => read_skill(skills, &text("name")),
        _ => Err("No such project tool.".to_string()),
    };
    match answer {
        Ok(output) => ToolResult { call, ok: true, output, images: Vec::new() },
        Err(reason) => ToolResult { call, ok: false, output: json!({ "error": reason }), images: Vec::new() },
    }
}

/// Where a path asked for reads from: inside the project, or inside a
/// listed skill's repository. Never Studio's own folder, and never
/// anywhere else.
fn readable(folder: &Path, skills: &[Skill], asked: &str) -> Result<PathBuf, String> {
    let asked = asked.trim();
    let path = if asked.is_empty() { folder.to_path_buf() } else if Path::new(asked).is_absolute() { PathBuf::from(asked) } else { folder.join(asked) };
    let real = path.canonicalize().map_err(|_| format!("Not found: {}", path.display()))?;
    let project = folder.canonicalize().map_err(|_| "The project's folder is gone.".to_string())?;
    if let Ok(inside) = real.strip_prefix(&project) {
        // Downloads waiting to be filed are the one part of Studio's own
        // folder the assistant looks at.
        let staged = Path::new(protocol::PROJECT_DIR).join("cache").join("research");
        if let Some(Component::Normal(first)) = inside.components().next()
            && OWN_FOLDERS.iter().any(|own| first == *own)
            && !inside.starts_with(&staged)
        {
            return Err("That is Studio's own folder, which no step needs.".into());
        }
        return Ok(real);
    }
    let in_skills = skills.iter().any(|skill| skill.root.canonicalize().is_ok_and(|root| real.starts_with(root)));
    if in_skills {
        return Ok(real);
    }
    Err("Only the project and the skills' repositories can be read.".into())
}

fn list_files(folder: &Path, skills: &[Skill], asked: &str) -> Result<Value, String> {
    let dir = readable(folder, skills, asked)?;
    if !dir.is_dir() {
        return Err(format!("Not a folder: {}", dir.display()));
    }
    let mut entries: Vec<(String, bool, u64)> = std::fs::read_dir(&dir)
        .map_err(|error| format!("Could not read {}: {error}", dir.display()))?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".git" || OWN_FOLDERS.contains(&name.as_str()) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            Some((name, meta.is_dir(), if meta.is_dir() { 0 } else { meta.len() }))
        })
        .collect();
    entries.sort();
    let listed: Vec<Value> = entries
        .into_iter()
        .map(|(name, is_folder, size)| if is_folder { json!({ "name": name, "folder": true }) } else { json!({ "name": name, "bytes": size }) })
        .collect();
    Ok(json!({ "folder": dir.display().to_string(), "entries": listed }))
}

/// A request's report holds its record as JSON; that is what is read,
/// laid out one value to a line, with every image or clip embedded in it
/// left out, since a record can carry megabytes of them.
fn report_record(text: &str) -> Option<String> {
    let start = text.find(r#"<script type="application/json" id="request">"#)?;
    let body = &text[start..];
    let open = body.find('>')? + 1;
    let close = body.find("</script>")?;
    let json = body[open..close].replace("<\\/", "</");
    let Ok(mut record) = serde_json::from_str::<Value>(&json) else { return Some(json) };
    leave_out_embedded(&mut record);
    serde_json::to_string_pretty(&record).ok()
}

/// A request's report's record, as JSON, with nothing left out.
pub(crate) fn record_of(text: &str) -> Option<Value> {
    let start = text.find(r#"<script type="application/json" id="request">"#)?;
    let body = &text[start..];
    let open = body.find('>')? + 1;
    let close = body.find("</script>")?;
    serde_json::from_str(&body[open..close].replace("<\\/", "</")).ok()
}

fn leave_out_embedded(value: &mut Value) {
    match value {
        Value::String(text) if text.starts_with("data:") => {
            *text = format!("[an embedded file of {} characters, left out; the report shows it]", text.len());
        }
        Value::Array(items) => items.iter_mut().for_each(leave_out_embedded),
        Value::Object(fields) => fields.values_mut().for_each(leave_out_embedded),
        _ => {}
    }
}

fn read_file(folder: &Path, skills: &[Skill], asked: &str, from_line: u64) -> Result<Value, String> {
    let path = readable(folder, skills, asked)?;
    if path.is_dir() {
        return Err(format!("{} is a folder; list it with list_files.", path.display()));
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let Ok(mut text) = String::from_utf8(bytes) else {
        return Err(format!("{} is not text. Look at an image with look_at_file.", path.display()));
    };
    let mut record = false;
    if path.extension().is_some_and(|extension| extension == "html")
        && let Some(json) = report_record(&text)
    {
        text = json;
        record = true;
    }
    let from = from_line.max(1) as usize;
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::new();
    let mut last = from - 1;
    for line in lines.iter().skip(from - 1) {
        if out.len() + line.len() + 1 > MOST_TEXT && !out.is_empty() {
            break;
        }
        // A line longer than a reply is cut, never sent whole.
        if line.len() > MOST_TEXT {
            let mut end = MOST_TEXT;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            out.push_str(&line[..end]);
            out.push_str(&format!(" [line {} is {} characters; the rest is left out]\n", last + 1, line.len()));
        } else {
            out.push_str(line);
            out.push('\n');
        }
        last += 1;
    }
    let mut answer = json!({ "path": path.display().to_string(), "text": out, "lines": format!("{from}-{last} of {}", lines.len()) });
    if record {
        answer["note"] = json!("This is the request's record, as JSON, from its report.");
    }
    if last < lines.len() {
        answer["more"] = json!(format!("Read on with from_line {}.", last + 1));
    }
    Ok(answer)
}

fn look(folder: &Path, skills: &[Skill], call: String, asked: &str) -> ToolResult {
    let failed = |call: String, reason: String| ToolResult { call, ok: false, output: json!({ "error": reason }), images: Vec::new() };
    let path = match readable(folder, skills, asked) {
        Ok(path) => path,
        Err(reason) => return failed(call, reason),
    };
    let extension = path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif") {
        return failed(call, format!("{} is not an image look_at_file can show: PNG, JPEG, WebP, or GIF.", path.display()));
    }
    let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    if size > MOST_IMAGE {
        return failed(call, format!("{} is {} MB, more than look_at_file sends; look at a smaller copy.", path.display(), size / (1024 * 1024)));
    }
    match std::fs::read(&path) {
        Ok(bytes) if is_lfs_pointer(&bytes) => failed(call, format!("{} is in Git LFS and has not been downloaded; run git lfs pull.", path.display())),
        Ok(bytes) => ToolResult {
            call,
            ok: true,
            output: json!({ "path": path.display().to_string(), "bytes": bytes.len() }),
            images: vec![assistant::image_base64(&bytes)],
        },
        Err(error) => failed(call, format!("Could not read {}: {error}", path.display())),
    }
}

fn is_lfs_pointer(bytes: &[u8]) -> bool {
    bytes.starts_with(b"version https://git-lfs.github.com/spec/")
}

/// The agent's folder: `agent/`, or `working/` in a project that has not
/// moved it yet.
fn agent_folder(folder: &Path) -> PathBuf {
    let former = folder.join("working");
    if !folder.join("agent").is_dir() && former.is_dir() { former } else { folder.join("agent") }
}

/// A name the agent gives a file, as one plain file name.
fn plain_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    let mut parts = Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(only)), None) if !name.starts_with('.') => Ok(only.to_string_lossy().to_string()),
        _ => Err(format!("\"{name}\" is not a plain file name.")),
    }
}

fn write_file(folder: &Path, to: &str, name: &str, text: &str, about: &str) -> Result<Value, String> {
    let name = plain_name(name)?;
    match to {
        "brief" => {
            let dir = folder.join("brief");
            std::fs::create_dir_all(&dir).map_err(|error| format!("Could not make brief/: {error}"))?;
            let path = dir.join(&name);
            std::fs::write(&path, text).map_err(|error| format!("Could not write {}: {error}", path.display()))?;
            Ok(json!({ "path": path.display().to_string() }))
        }
        "agent" => {
            if about.trim().is_empty() {
                return Err("Say what the file is, in about, for the index.".into());
            }
            let dir = agent_folder(folder);
            std::fs::create_dir_all(&dir).map_err(|error| format!("Could not make {}: {error}", dir.display()))?;
            let file = format!("{}-{name}", local_stamp());
            let path = dir.join(&file);
            if path.exists() {
                return Err(format!("{} exists, and the agent's folder is only added to; give another name.", path.display()));
            }
            std::fs::write(&path, text).map_err(|error| format!("Could not write {}: {error}", path.display()))?;
            let index = dir.join("index.md");
            let before = std::fs::read_to_string(&index).unwrap_or_default();
            let mut line = String::new();
            if !before.is_empty() && !before.ends_with('\n') {
                line.push('\n');
            }
            // A list line after a table needs a blank line between them.
            if before.lines().last().is_some_and(|last| last.trim_start().starts_with('|')) {
                line.push('\n');
            }
            line.push_str(&format!("- `{file}`: {}\n", about.trim()));
            std::fs::write(&index, format!("{before}{line}")).map_err(|error| format!("Could not update {}: {error}", index.display()))?;
            Ok(json!({ "path": path.display().to_string(), "index": index.display().to_string() }))
        }
        _ => Err("Write only to brief or agent.".into()),
    }
}

fn read_skill(skills: &[Skill], name: &str) -> Result<Value, String> {
    let skill = skills.iter().find(|skill| skill.name == name.trim()).ok_or_else(|| format!("No skill named \"{}\" is listed.", name.trim()))?;
    let body = std::fs::read_to_string(skill.folder.join("SKILL.md")).map_err(|error| format!("Could not read the skill: {error}"))?;
    Ok(json!({
        "note": format!("In this project, this skill {RENDERS_HERE}: a model is called only through request.py and the plan card."),
        "folder": skill.folder.display().to_string(),
        "repository": skill.root.display().to_string(),
        "skill": body,
    }))
}

/// The local time as a file name starts with it: `2026-11-14_21-12`.
pub(crate) fn local_stamp() -> String {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as libc::time_t)
        .unwrap_or(0);
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    // Safety: `localtime_r` writes only into `parts`, which lives on this
    // frame, and reads only `time`.
    let done = unsafe { libc::localtime_r(&time, &mut parts) };
    if done.is_null() {
        return "0000-00-00_00-00".into();
    }
    format!("{:04}-{:02}-{:02}_{:02}-{:02}", parts.tm_year + 1900, parts.tm_mon + 1, parts.tm_mday, parts.tm_hour, parts.tm_min)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("skills-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn project(name: &str) -> PathBuf {
        let folder = scratch(name).join("film");
        for dir in ["library", "requests", "brief", "agent", ".spumcake"] {
            std::fs::create_dir_all(folder.join(dir)).unwrap();
        }
        folder
    }

    fn skill_repo(root: &Path) -> PathBuf {
        let repo = root.join("skills-repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        for (path, front) in [
            ("skills/generate-keyframe", "name: generate-keyframe\ndescription: Make keyframes."),
            ("skills/run-prompt", "name: run-prompt\ndescription: Use whenever a written prompt is to be rendered."),
            (".project/skills/workflow/minimax-h3-workflows", "name: minimax-h3-workflows\ndescription: What is true of H3.\nimplements: [image-to-video, run-prompt]"),
        ] {
            std::fs::create_dir_all(repo.join(path)).unwrap();
            std::fs::write(repo.join(path).join("SKILL.md"), format!("---\n{front}\n---\n\n# Body of {path}\n")).unwrap();
        }
        std::fs::create_dir_all(repo.join("prompt-skills")).unwrap();
        std::fs::write(repo.join("prompt-skills/minimax-h3-fal-video.md"), "One English block.\n").unwrap();
        repo
    }

    /// Catches a project tool reading outside the project or the skills,
    /// or Studio's own folder, which no step needs.
    #[test]
    fn reads_stay_inside_the_project_and_the_skills() {
        let folder = project("reads");
        let repo = skill_repo(folder.parent().unwrap());
        let skills = find_skills(&[repo.clone()]);
        std::fs::write(folder.join("brief/001-film.html"), "brief").unwrap();
        assert!(read_file(&folder, &skills, "brief/001-film.html", 1).is_ok());
        assert!(read_file(&folder, &skills, &repo.join("prompt-skills/minimax-h3-fal-video.md").display().to_string(), 1).is_ok());
        assert!(read_file(&folder, &skills, "../outside.txt", 1).is_err());
        assert!(read_file(&folder, &skills, "/etc/hostname", 1).is_err());
        std::fs::write(folder.join(".spumcake/project.json"), "{}").unwrap();
        assert!(read_file(&folder, &skills, ".spumcake/project.json", 1).is_err());
        let listed = list_files(&folder, &skills, "").unwrap();
        assert!(!listed.to_string().contains(".spumcake"));
    }

    /// Catches a skill missed in a repository's `.project/skills/`, as a
    /// workflow skill lives, and a skill listed without the setup's note.
    #[test]
    fn every_skill_is_found_and_renders_through_request_py() {
        let folder = project("found");
        let repo = skill_repo(folder.parent().unwrap());
        let skills = find_skills(&[repo.clone()]);
        let names: Vec<&str> = skills.iter().map(|skill| skill.name.as_str()).collect();
        assert_eq!(names, ["minimax-h3-workflows", "generate-keyframe", "run-prompt"]);
        assert_eq!(skills[0].implements, ["image-to-video", "run-prompt"]);
        assert_eq!(skills[0].root, repo);
        let group = tool_group(&folder, &skills);
        for name in ["run-prompt", "generate-keyframe", "minimax-h3-workflows"] {
            assert!(group.instructions.lines().any(|line| line.starts_with(&format!("- {name}:")) && line.contains(RENDERS_HERE)), "{name}");
        }
        assert!(group.instructions.contains("request.py is the one renderer"));
        let read = read_skill(&skills, "run-prompt").unwrap();
        assert!(read["note"].as_str().unwrap().contains(RENDERS_HERE));
        assert!(read["skill"].as_str().unwrap().contains("# Body of skills/run-prompt"));
    }

    /// Catches the agent writing where only Skills' scripts write, or
    /// overwriting a file in the agent's folder, which is only added to.
    #[test]
    fn the_agent_writes_only_brief_and_new_agent_files() {
        let folder = project("writes");
        assert!(write_file(&folder, "library", "x.png", "x", "").is_err());
        assert!(write_file(&folder, "agent", "../library/x.png", "x", "about").is_err());
        assert!(write_file(&folder, "agent", "check.md", "x", "").is_err(), "an agent file needs its index line");
        let written = write_file(&folder, "agent", "check.md", "x", "A check.").unwrap();
        let path = PathBuf::from(written["path"].as_str().unwrap());
        assert!(path.starts_with(folder.join("agent")) && path.file_name().unwrap().to_string_lossy().ends_with("-check.md"));
        let index = std::fs::read_to_string(folder.join("agent/index.md")).unwrap();
        assert!(index.contains(&format!("`{}`: A check.", path.file_name().unwrap().to_string_lossy())));
        assert!(write_file(&folder, "agent", "check.md", "y", "Again.").is_err(), "the same minute's name is not overwritten");
        assert!(write_file(&folder, "brief", "001-film.html", "page", "").is_ok());
    }

    /// Catches a report read as its whole page, with every image inside it,
    /// instead of its record.
    #[test]
    fn a_report_reads_as_its_record() {
        let folder = project("record");
        std::fs::write(
            folder.join("requests/r.html"),
            "<html><img src=\"data:image/jpeg;base64,AAAA\"><script type=\"application/json\" id=\"request\">{\"id\": \"r\", \"prompt\": \"a <\\/b>\", \"shown\": [{\"embedded\": \"data:image/png;base64,QUJD\"}]}</script></html>",
        )
        .unwrap();
        let read = read_file(&folder, &[], "requests/r.html", 1).unwrap();
        let record: Value = serde_json::from_str(read["text"].as_str().unwrap()).unwrap();
        assert_eq!(record["prompt"], "a </b>");
        assert!(!read["text"].as_str().unwrap().contains("base64"), "an embedded file was sent");
    }

    /// Catches an LFS pointer sent to the model as if it were the image.
    #[test]
    fn an_lfs_pointer_is_not_an_image() {
        let folder = project("lfs");
        std::fs::write(folder.join("library/a.png"), "version https://git-lfs.github.com/spec/v1\noid sha256:00\nsize 1\n").unwrap();
        let result = look(&folder, &[], "c".into(), "library/a.png");
        assert!(!result.ok && result.images.is_empty());
        std::fs::write(folder.join("library/b.png"), b"\x89PNG\r\n\x1a\nrest").unwrap();
        let result = look(&folder, &[], "c".into(), "library/b.png");
        assert!(result.ok && result.images.len() == 1);
    }

    /// Catches what only Last order's own files show: every report reads
    /// in replies of at most `MOST_TEXT` with no embedded file; its images
    /// can be looked at; and the skills a setup lists, linked or not, are
    /// found with the setup's note, their repositories readable, as the H3
    /// prompt skill is read before a shot is planned. Runs when
    /// LAST_ORDER names a copy of Last order, and
    /// SPUMCAKE_SKILL_FOLDERS the skill folders, separated by `:`.
    #[test]
    fn last_order_reads_as_it_stands() {
        let Ok(folder) = std::env::var("LAST_ORDER") else { return };
        let folder = PathBuf::from(folder);
        let folders: Vec<String> = std::env::var("SPUMCAKE_SKILL_FOLDERS").unwrap_or_default().split(':').filter(|f| !f.is_empty()).map(String::from).collect();
        let skills = find_skills(&folders.iter().map(PathBuf::from).collect::<Vec<_>>());
        let mut reports = 0;
        for entry in std::fs::read_dir(folder.join("requests")).unwrap().flatten() {
            let name = format!("requests/{}", entry.file_name().to_string_lossy());
            if !name.ends_with(".html") {
                continue;
            }
            reports += 1;
            let mut from = 1;
            loop {
                let read = read_file(&folder, &skills, &name, from).unwrap();
                let text = read["text"].as_str().unwrap();
                assert!(text.len() <= MOST_TEXT + 200, "{name} from line {from}: {} characters", text.len());
                assert!(!text.contains("base64,"), "{name} sends an embedded file");
                match read["more"].as_str() {
                    Some(more) => from = more.trim_start_matches("Read on with from_line ").trim_end_matches('.').parse().unwrap(),
                    None => break,
                }
            }
        }
        assert!(reports > 400, "{reports} reports read");
        let jpeg = std::fs::read_dir(folder.join("sources")).unwrap().flatten().find(|e| e.path().extension().is_some_and(|x| x == "jpg")).unwrap();
        let looked = look(&folder, &skills, "c".into(), &format!("sources/{}", jpeg.file_name().to_string_lossy()));
        assert!(looked.ok && looked.images.len() == 1, "{:?}", looked.output);
        if folders.is_empty() {
            return;
        }
        let group = tool_group(&folder, &skills);
        for name in ["run-prompt", "image-to-video", "open-router-generate-images", "generate-keyframe"] {
            assert!(group.instructions.lines().any(|line| line.starts_with(&format!("- {name}:")) && line.contains(RENDERS_HERE)), "{name} is not listed with the note");
        }
        let h3 = skills.iter().find(|skill| skill.name == "minimax-h3-workflows").expect("the H3 workflow skill");
        let prompt_skill = h3.root.join("prompt-skills/minimax-h3-fal-video.md");
        assert!(read_file(&folder, &skills, &prompt_skill.display().to_string(), 1).is_ok(), "the H3 prompt skill cannot be read");
        let keyframe = skills.iter().find(|skill| skill.name == "generate-keyframe").unwrap();
        let roles = keyframe.folder.join("../../docs/prompt-roles.md");
        assert!(read_file(&folder, &skills, &roles.display().to_string(), 1).is_ok(), "a skill's relative link cannot be read");
    }
}
