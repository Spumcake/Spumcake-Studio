//! The Prompt page's runner duties, in a Skills project: what the card
//! shows of a cell, and what running it does, through Spumcake-Skills' own
//! scripts, as the Request page's draft in the lifecycle contract gives
//! them to this module.
//!
//! - A cell that is a chain step is read from `canon.sql` with
//!   `pipeline.py step --show`, when the project opens, when `canon.sql`
//!   changes, and before each spec; the read is held here only, never
//!   written or sent to the workspace.
//! - A run makes a spec, by `pipeline.py fill` for a step or from the
//!   draft, and goes through the plan card, as the assistant's requests do.
//! - The cost line comes from `request.py plan`, offline, asked once the
//!   cell has stood still for a moment.
//! - A reference is "Not canon" unless its file is a published artifact's,
//!   read from `canon.sql`: a read, not a judgement of what is out of date.
//! - A cell's rounds are read from `requests/` and the pending records, and
//!   sent to its workspace with `takes`.

use protocol::{Cell, CellPart, ItemRef, Round, RoundState, SentReference, StepRef, TakeFile};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use crate::workspace::{EndpointKind, Settings};

/// How long a cell stands still before its cost is asked again.
const COST_PAUSE: Duration = Duration::from_millis(700);

/// What the project's canon holds: every artifact's file, and which are
/// published.
#[derive(Debug, Default, Clone)]
pub(crate) struct Canon {
    /// Artifact name to its file, relative to the project.
    pub files: HashMap<String, String>,
    /// The files of published artifacts.
    pub published: HashSet<String>,
}

/// The living pipeline: the highest-numbered folder in `pipeline/`, as the
/// owner set on 2026-11-16.
pub(crate) fn living_pipeline(folder: &Path) -> Option<String> {
    std::fs::read_dir(folder.join("pipeline"))
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().filter(|name| name.chars().all(|c| c.is_ascii_digit())).map(String::from))
        .max_by_key(|name| name.parse::<u32>().unwrap_or(0))
}

/// Runs one of Skills' scripts that reads, in the project, and returns
/// what it printed.
fn script(scripts: &Path, folder: &Path, setup: &[(String, String)], name: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new("python3")
        .arg(scripts.join(name))
        .args(args)
        .current_dir(folder)
        .envs(setup.iter().filter(|(var, _)| !var.ends_with("_KEY")).map(|(var, value)| (var.as_str(), value.as_str())))
        .output()
        .map_err(|error| format!("Could not run {name}: {error}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The canon, read through `pipeline.py sql --read-only`.
pub(crate) fn read_canon(scripts: &Path, folder: &Path, setup: &[(String, String)], pipeline: &str) -> Result<Canon, String> {
    let rows = script(scripts, folder, setup, "pipeline.py", &["sql", pipeline, "SELECT id, file, published FROM artifact", "--read-only"])?;
    let mut canon = Canon::default();
    for row in rows.lines() {
        let cells: Vec<&str> = row.split(" | ").collect();
        if cells.len() < 3 || cells[1].is_empty() {
            continue;
        }
        canon.files.insert(cells[0].to_string(), cells[1].to_string());
        if !cells[2].trim().is_empty() {
            canon.published.insert(cells[1].to_string());
        }
    }
    Ok(canon)
}

/// A chain step, as `pipeline.py step --show` prints it.
pub(crate) fn read_step(scripts: &Path, folder: &Path, setup: &[(String, String)], pipeline: &str, step: &StepRef) -> Result<Value, String> {
    let text = script(scripts, folder, setup, "pipeline.py", &["step", pipeline, &step.chain, &step.step, "--show"])?;
    serde_json::from_str(&text).map_err(|error| format!("The step could not be read: {error}"))
}

/// The cell the card shows for a step: the declared pointer, filled with
/// the step's template, references, endpoint, and settings, read now.
pub(crate) fn step_cell(declared: &Cell, step: &Value, canon: &Canon) -> Cell {
    let mut cell = declared.clone();
    let template = step.get("template").and_then(Value::as_str).unwrap_or("").to_string();
    let mut input = vec![CellPart::Text { text: template }];
    let refs: Vec<Value> = step.get("refs").and_then(Value::as_str).and_then(|refs| serde_json::from_str(refs).ok()).unwrap_or_default();
    for reference in refs {
        let text = |key: &str| reference.get(key).and_then(Value::as_str).map(String::from);
        let path = text("file").or_else(|| text("artifact").and_then(|artifact| canon.files.get(&artifact).cloned()));
        if let Some(path) = path {
            input.push(CellPart::Path { media_type: media_type(&path).to_string(), path, role: text("role"), field: text("field") });
        }
    }
    cell.input = input;
    let endpoint: Value = step.get("endpoint").and_then(Value::as_str).and_then(|endpoint| serde_json::from_str(endpoint).ok()).unwrap_or(Value::Null);
    cell.model = endpoint.get("model").and_then(Value::as_str).map(String::from);
    if let Some(settings) = endpoint.get("settings") {
        cell.settings = Some(settings.clone());
    }
    cell
}

/// A file's media type, from its extension.
pub(crate) fn media_type(path: &str) -> &'static str {
    match Path::new(path).extension().map(|extension| extension.to_string_lossy().to_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("md" | "txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

/// The references of a cell, in order, as files relative to the project.
fn references(cell: &Cell) -> Vec<(String, Option<String>, Option<String>)> {
    cell.input
        .iter()
        .filter_map(|part| match part {
            CellPart::Path { path, role, field, .. } => Some((path.clone(), role.clone(), field.clone())),
            _ => None,
        })
        .collect()
}

/// The references, by index, whose file is not a published artifact's.
pub(crate) fn not_canon(cell: &Cell, canon: &Canon) -> Vec<usize> {
    references(cell).iter().enumerate().filter(|(_, (path, _, _))| !canon.published.contains(path)).map(|(index, _)| index).collect()
}

/// The kind of request `request.py` makes for a cell on this endpoint.
fn request_kind(settings: &Settings, cell: &Cell) -> Result<&'static str, String> {
    let endpoint = cell.endpoint.as_deref().and_then(|id| settings.endpoint(id)).ok_or("This cell has no endpoint. Choose one for it.")?;
    let image = cell.kind == protocol::IMAGE_KIND;
    let video = cell.kind == protocol::VIDEO_KIND;
    let has_references = !references(cell).is_empty();
    Ok(match endpoint.kind {
        EndpointKind::Fal => "fal",
        EndpointKind::Openrouter if image => "openrouter-image",
        EndpointKind::Openrouter if !video => "openrouter-text",
        EndpointKind::Openai if endpoint.address.contains("openai.com") && image => {
            if has_references { "openai-image-edit" } else { "openai-image" }
        }
        EndpointKind::Openai if endpoint.address.contains("openai.com") && !video => "openai-text",
        _ => return Err(format!("“{}” is not an endpoint Spumcake-Skills' request.py runs, so this cell cannot run in a Skills project.", endpoint.name)),
    })
}

/// A name for a request, from the cell's prompt.
fn request_name(cell: &Cell) -> String {
    let words: String = cell.prompt().split_whitespace().take(5).collect::<Vec<_>>().join(" ").to_lowercase();
    let slug: String = words.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let slug = slug.split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-");
    if slug.is_empty() { String::from("take") } else { slug }
}

/// The spec a draft cell runs as: its prompt, its references in order with
/// their roles and fields, its endpoint and settings, and the setup's own
/// `setup` object naming the cell, which `request.py` keeps in the report.
pub(crate) fn draft_spec(settings: &Settings, cell: &Cell, at: &ItemRef) -> Result<Value, String> {
    for part in &cell.input {
        if let CellPart::Item { .. } = part {
            return Err(String::from("A reference from a workspace is filed into the project first; add the filed file instead."));
        }
    }
    let references: Vec<Value> = references(cell)
        .into_iter()
        .map(|(path, role, field)| {
            if path.starts_with("work/") {
                return Err(format!("{path} is in work/, which is never an input; file it first."));
            }
            let mut reference = json!({ "file": path });
            if let Some(role) = role {
                reference["role"] = json!(role);
            }
            if let Some(field) = field {
                reference["field"] = json!(field);
            }
            Ok(reference)
        })
        .collect::<Result<_, String>>()?;
    Ok(json!({
        "spec": 1,
        "name": request_name(cell),
        "endpoint": { "kind": request_kind(settings, cell)?, "model": cell.model, "settings": cell.settings.clone().unwrap_or(json!({})) },
        "prompt": cell.prompt(),
        "references": references,
        "setup": { "cell": at },
    }))
}

/// The spec a step's cell runs as, filled from canon by `pipeline.py fill`,
/// with the setup's object naming the cell.
pub(crate) fn step_spec(scripts: &Path, folder: &Path, setup: &[(String, String)], pipeline: &str, step: &StepRef, at: &ItemRef) -> Result<Value, String> {
    let text = script(scripts, folder, setup, "pipeline.py", &["fill", pipeline, &step.chain, &step.step])?;
    let mut spec: Value = serde_json::from_str(&text).map_err(|error| format!("The step's spec was not JSON: {error}"))?;
    spec["setup"] = json!({ "cell": at });
    Ok(spec)
}

/// A name for the spec's request that no report or pending record of this
/// minute has, so Try again within a minute is a new request: `request.py`
/// names a request by the minute and its name.
pub(crate) fn unique_name(folder: &Path, spec: &mut Value) {
    let base = spec.get("name").and_then(Value::as_str).unwrap_or("take").to_string();
    let slug = |name: &str| {
        let lowered: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        lowered.split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-")
    };
    let stamp = crate::skills_setup::local_stamp();
    let taken = |name: &str| {
        let id = format!("{stamp}-{}", slug(name));
        folder.join("requests").join(format!("{id}.html")).exists() || folder.join("requests").join(format!("{id}.pending.json")).exists()
    };
    let mut name = base.clone();
    let mut n = 2;
    while taken(&name) {
        name = format!("{base}-{n}");
        n += 1;
    }
    spec["name"] = json!(name);
}

/// Writes a spec where Studio keeps its own scratch files, inside the
/// project so `request.py` finds the project from it. Returns its path,
/// relative to the project.
pub(crate) fn write_spec(folder: &Path, cell: &str, spec: &Value) -> Result<String, String> {
    let dir = folder.join(protocol::PROJECT_DIR).join("cache").join("specs");
    std::fs::create_dir_all(&dir).map_err(|error| format!("Could not make {}: {error}", dir.display()))?;
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|elapsed| elapsed.as_millis()).unwrap_or(0);
    let safe: String = cell.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let name = format!("{safe}-{stamp}.json");
    std::fs::write(dir.join(&name), serde_json::to_string_pretty(spec).unwrap_or_default()).map_err(|error| format!("Could not write the spec: {error}"))?;
    Ok(format!("{}/cache/specs/{name}", protocol::PROJECT_DIR))
}

/// The cost line under the card, from a plan's estimate and the service.
pub(crate) fn cost_line(plan: &Value, service: &str) -> String {
    let estimate = plan.get("estimate").and_then(Value::as_str).unwrap_or("");
    let shown = match estimate.strip_prefix("about ") {
        Some(rest) => format!("About {rest}"),
        None => String::from("Cost unknown"),
    };
    if service.is_empty() { shown } else { format!("{shown} · {service}") }
}

/// Seconds since the Unix epoch, from a record's `2026-11-14T13:30:00Z`.
fn epoch(time: &str) -> Option<u64> {
    let digits: Vec<u64> = time.split(|c: char| !c.is_ascii_digit()).filter(|part| !part.is_empty()).filter_map(|part| part.parse().ok()).collect();
    let [year, month, day, hour, minute, second] = digits.get(..6)?.try_into().ok()?;
    // Days from 1970-01-01 to the date, by the civil-calendar formula.
    let (y, m) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Every round a cell made: each report in `requests/` whose record names
/// the cell in its `setup`, or its step; and each pending record, oldest
/// first, numbered from 1.
/// What `rounds` keeps of each report, by its file and time, so a report
/// is read once, not on every change to `requests/`.
pub(crate) type Reports = HashMap<PathBuf, (Option<SystemTime>, Option<Value>)>;

/// A report's record, cut to what a round needs.
fn slim(record: Value) -> Value {
    let keep = |key: &str| record.get(key).cloned().unwrap_or(Value::Null);
    let references: Vec<Value> = record
        .get("references")
        .and_then(Value::as_array)
        .map(|references| references.iter().map(|reference| json!({ "file": reference.get("file"), "role": reference.get("role") })).collect())
        .unwrap_or_default();
    json!({
        "setup": keep("setup"), "chain": keep("chain"), "step": keep("step"), "started": keep("started"),
        "outputs": keep("outputs"), "prompt": keep("prompt"), "references": references,
        "sent": { "model": record.pointer("/sent/model").cloned().unwrap_or(Value::Null) },
        "endpoint": { "model": record.pointer("/endpoint/model").cloned().unwrap_or(Value::Null) },
    })
}

pub(crate) fn rounds(folder: &Path, cell: &str, step: Option<&StepRef>, canon: &Canon, reports: &mut Reports) -> Vec<Round> {
    let mut found: Vec<(u64, Round)> = Vec::new();
    let names_cell = |record: &Value| {
        record.pointer("/setup/cell/item").and_then(Value::as_str) == Some(cell)
            || step.is_some_and(|step| {
                record.get("chain").and_then(Value::as_str) == Some(step.chain.as_str()) && record.get("step").and_then(Value::as_str) == Some(step.step.as_str())
            })
    };
    for entry in std::fs::read_dir(folder.join("requests")).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(id) = name.strip_suffix(".pending.json") {
            let Some(pending) = std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()) else { continue };
            let spec = pending.get("spec").cloned().unwrap_or(Value::Null);
            if !names_cell(&spec) {
                continue;
            }
            let started = pending.get("started").and_then(Value::as_str).and_then(epoch).unwrap_or(0);
            found.push((started, round_of(id, &spec, RoundState::Pending, started, &[], None, None)));
        } else if let Some(id) = name.strip_suffix(".html") {
            let time = modified(&path);
            let known = reports.get(&path).filter(|(at, _)| *at == time).map(|(_, record)| record.clone());
            let record = match known {
                Some(record) => record,
                None => {
                    let record = std::fs::read_to_string(&path).ok().as_deref().and_then(crate::skills_setup::record_of).map(slim);
                    reports.insert(path.clone(), (time, record.clone()));
                    record
                }
            };
            let Some(record) = record else { continue };
            if !names_cell(&record) {
                continue;
            }
            let started = record.get("started").and_then(Value::as_str).and_then(epoch).unwrap_or(0);
            let outputs: Vec<String> = record.get("outputs").and_then(Value::as_array).map(|outputs| outputs.iter().filter_map(|output| output.as_str().map(String::from)).collect()).unwrap_or_default();
            let failed = outputs.is_empty();
            let report = format!("requests/{name}");
            let state = if failed { RoundState::Failed } else { RoundState::Done };
            let error = failed.then(|| String::from("Nothing came back. Open its report to see why."));
            found.push((started, round_of(id, &record, state, started, &outputs, error, Some(report))));
        }
    }
    found.sort_by_key(|(started, round)| (*started, round.request.clone()));
    found
        .into_iter()
        .enumerate()
        .map(|(index, (_, mut round))| {
            round.number = index as u32 + 1;
            for take in &mut round.takes {
                take.canon = canon.published.contains(&take.path);
            }
            round
        })
        .collect()
}

fn round_of(id: &str, record: &Value, state: RoundState, started: u64, outputs: &[String], error: Option<String>, report: Option<String>) -> Round {
    let model = record
        .pointer("/sent/model")
        .or_else(|| record.pointer("/endpoint/model"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let references = record
        .get("references")
        .and_then(Value::as_array)
        .map(|references| {
            references
                .iter()
                .filter_map(|reference| {
                    let path = reference.get("file").and_then(Value::as_str)?.to_string();
                    Some(SentReference { path, role: reference.get("role").and_then(Value::as_str).map(String::from) })
                })
                .collect()
        })
        .unwrap_or_default();
    Round {
        request: id.to_string(),
        number: 0,
        model,
        prompt: record.get("prompt").and_then(Value::as_str).unwrap_or("").to_string(),
        references,
        left_out: Vec::new(),
        state,
        started: (started > 0).then_some(started),
        count: None,
        takes: outputs.iter().map(|path| TakeFile { path: path.clone(), media_type: media_type(path).to_string(), canon: false }).collect(),
        error,
        report,
    }
}

/// The card's cost, asked once a cell has stood still, in the background.
#[derive(Default)]
pub(crate) struct Costs {
    asked: HashMap<String, Cost>,
}

struct Cost {
    /// What the cell was when last seen, so a change asks again.
    shape: u64,
    seen: Instant,
    line: Option<String>,
    waiting: Option<std::sync::mpsc::Receiver<Option<String>>>,
    /// The plan was asked for this shape: asked once, even when it fails.
    asked: bool,
}

fn shape_of(cell: &Cell) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&(&cell.input, &cell.settings, &cell.model, &cell.endpoint, &cell.step)).unwrap_or_default().hash(&mut hasher);
    hasher.finish()
}

impl Costs {
    /// The cost line to show for a cell now. `plan` works out its plan, and
    /// runs on another thread once the cell has stood still.
    pub(crate) fn line(&mut self, cell: &Cell, plan: impl FnOnce() -> Option<String> + Send + 'static) -> Option<String> {
        let shape = shape_of(cell);
        let cost = self.asked.entry(cell.id.clone()).or_insert(Cost { shape, seen: Instant::now(), line: None, waiting: None, asked: false });
        if cost.shape != shape {
            cost.shape = shape;
            cost.seen = Instant::now();
            cost.waiting = None;
            cost.asked = false;
        }
        if let Some(waiting) = &cost.waiting
            && let Ok(line) = waiting.try_recv()
        {
            // A plan that could not be made says so: never blank, never zero.
            cost.line = Some(line.unwrap_or_else(|| String::from("Cost unknown")));
            cost.waiting = None;
        }
        if cost.seen.elapsed() >= COST_PAUSE && !cost.asked {
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = send.send(plan());
            });
            cost.waiting = Some(receive);
            cost.asked = true;
        }
        cost.line.clone()
    }
}

/// The step a cell points to, the living pipeline's.
pub(crate) fn step_of(cell: &Cell) -> Option<&StepRef> {
    cell.step.as_ref()
}

/// Where a `pipeline.py step` command recorded a step that a request ran:
/// the chain, the step, and the request, from its arguments.
pub(crate) fn recorded_step(argv: &[String]) -> Option<(StepRef, String)> {
    let at = argv.iter().position(|arg| arg.ends_with("pipeline.py"))?;
    if argv.get(at + 1).map(String::as_str) != Some("step") {
        return None;
    }
    let chain = argv.get(at + 3)?.clone();
    let step = argv.get(at + 4)?.clone();
    let request = argv.windows(2).find(|pair| pair[0] == "--request").map(|pair| pair[1].clone())?;
    Some((StepRef { chain, step }, request))
}

/// The cell a request was made from, by its record's `setup`.
pub(crate) fn cell_of_request(folder: &Path, request: &str) -> Option<ItemRef> {
    let text = std::fs::read_to_string(folder.join("requests").join(format!("{request}.html"))).ok()?;
    let record = crate::skills_setup::record_of(&text)?;
    serde_json::from_value(record.pointer("/setup/cell")?.clone()).ok()
}

/// The command that files a workspace's file into `prompts/resources/`:
/// the first `from` is the workspace file in `work/`, recorded as where it
/// came from, and the rest the library files it links, as its inputs.
pub(crate) fn resource_command(path: &str, from: &[String]) -> Vec<String> {
    let name = Path::new(path).file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_else(|| String::from("resource"));
    let mut argv = vec![String::from("request.py"), String::from("resource"), path.to_string(), String::from("--name"), name];
    for file in from {
        argv.push(String::from("--from"));
        argv.push(file.clone());
    }
    argv
}

/// What a person's edit to a step's cell changes, each as the step's own
/// field, or None when unchanged: the template; the references, each kept
/// as the step had it, with a reference the card cannot show (an artifact
/// with no file yet) kept in its place; and the endpoint with its settings.
pub(crate) fn step_edits(read: &Value, canon: &Canon, input: &[CellPart], settings: Option<&Value>) -> (Option<String>, Option<String>, Option<String>) {
    let old_template = read.get("template").and_then(Value::as_str).unwrap_or("");
    let template: String = input.iter().filter_map(|part| match part { CellPart::Text { text } => Some(text.as_str()), _ => None }).collect::<Vec<_>>().join("\n");
    let old: Vec<Value> = read.get("refs").and_then(Value::as_str).and_then(|refs| serde_json::from_str(refs).ok()).unwrap_or_default();
    let text = |reference: &Value, key: &str| reference.get(key).and_then(Value::as_str).map(String::from);
    let file_of = |reference: &Value| text(reference, "file").or_else(|| text(reference, "artifact").and_then(|artifact| canon.files.get(&artifact).cloned()));
    // What the card showed, in order: each reference that has a file.
    let shown: Vec<(String, Option<String>, Option<String>)> = old.iter().filter_map(|reference| Some((file_of(reference)?, text(reference, "role"), text(reference, "field")))).collect();
    let edited: Vec<(String, Option<String>, Option<String>)> = input
        .iter()
        .filter_map(|part| match part {
            CellPart::Path { path, role, field, .. } => Some((path.clone(), role.clone(), field.clone())),
            _ => None,
        })
        .collect();
    let refs = (edited != shown).then(|| {
        let mut used = vec![false; old.len()];
        let mut refs: Vec<Value> = Vec::new();
        for (path, role, field) in &edited {
            let kept = old.iter().enumerate().find(|(index, reference)| !used[*index] && file_of(reference).as_deref() == Some(path.as_str()));
            let mut reference = match kept {
                Some((index, reference)) => {
                    used[index] = true;
                    reference.clone()
                }
                None => json!({ "file": path }),
            };
            match role {
                Some(role) => reference["role"] = json!(role),
                None => {
                    if let Some(fields) = reference.as_object_mut() {
                        fields.remove("role");
                    }
                }
            }
            if let Some(field) = field {
                reference["field"] = json!(field);
            }
            refs.push(reference);
        }
        // A reference with no file to show stays as it was.
        for (index, reference) in old.iter().enumerate() {
            if !used[index] && file_of(reference).is_none() {
                refs.push(reference.clone());
            }
        }
        serde_json::to_string(&refs).unwrap_or_default()
    });
    let endpoint = settings.and_then(|settings| {
        let mut endpoint: Value = read.get("endpoint").and_then(Value::as_str).and_then(|text| serde_json::from_str(text).ok()).unwrap_or(json!({}));
        if endpoint.get("settings") == Some(settings) {
            return None;
        }
        endpoint["settings"] = settings.clone();
        Some(endpoint.to_string())
    });
    ((template != old_template).then_some(template), refs, endpoint)
}

/// What the card shows of one cell.
#[derive(Default)]
pub(crate) struct CardView {
    /// For a step, the cell filled from the step's read; None for a draft,
    /// which shows as declared.
    pub cell: Option<Cell>,
    pub not_canon: Vec<usize>,
    pub cost: Option<String>,
}

/// The Prompt page's runner state for the open project: canon and steps as
/// last read, held here only, and each cell's cost and rounds sent.
#[derive(Default)]
pub(crate) struct PromptPage {
    /// `canon.sql`'s time when canon and the steps were read.
    read_at: Option<SystemTime>,
    canon: Canon,
    /// Each step's read, or None when it could not be read, so a failing
    /// read is not tried again every frame.
    steps: HashMap<(String, String), Option<Value>>,
    reports: Reports,
    pub costs: Costs,
    /// For each cell, `requests/`'s time and canon's when its rounds were
    /// last sent: a publish re-marks which takes are canon.
    sent: HashMap<String, (SystemTime, Option<SystemTime>)>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

impl PromptPage {
    /// Reads canon again when `canon.sql` changed since the last read, and
    /// forgets the steps read before it.
    fn refresh(&mut self, folder: &Path, scripts: &Path, setup: &[(String, String)]) {
        let Some(pipeline) = living_pipeline(folder) else { return };
        let now = modified(&folder.join("pipeline").join(&pipeline).join("canon.sql"));
        if now.is_some() && now == self.read_at {
            return;
        }
        self.read_at = now;
        self.steps.clear();
        self.canon = read_canon(scripts, folder, setup, &pipeline).unwrap_or_default();
    }

    pub(crate) fn canon(&mut self, folder: &Path, scripts: &Path, setup: &[(String, String)]) -> Canon {
        self.refresh(folder, scripts, setup);
        self.canon.clone()
    }

    fn step(&mut self, folder: &Path, scripts: &Path, setup: &[(String, String)], step: &StepRef) -> Option<Value> {
        self.refresh(folder, scripts, setup);
        let key = (step.chain.clone(), step.step.clone());
        if let Some(known) = self.steps.get(&key) {
            return known.clone();
        }
        let read = living_pipeline(folder).and_then(|pipeline| read_step(scripts, folder, setup, &pipeline, step).ok());
        self.steps.insert(key, read.clone());
        read
    }

    /// Reads canon and the step again now, before a spec is made from them.
    pub(crate) fn forget(&mut self) {
        self.read_at = None;
    }

    /// What the card shows of a declared cell.
    pub(crate) fn card(&mut self, folder: &Path, scripts: &Path, setup: &[(String, String)], settings: &Settings, declared: &Cell, at: &ItemRef) -> CardView {
        let shown = match &declared.step {
            Some(step) => self.step(folder, scripts, setup, step).map(|read| step_cell(declared, &read, &self.canon)),
            None => None,
        };
        let canon = self.canon(folder, scripts, setup);
        let cell = shown.clone().unwrap_or_else(|| declared.clone());
        let not_canon = not_canon(&cell, &canon);
        // The cost is worked out from the spec the run would send.
        let spec = match &declared.step {
            Some(_) => None,
            None => draft_spec(settings, &cell, at).ok(),
        };
        let service = cell.endpoint.as_deref().and_then(|id| settings.endpoint(id)).map(|endpoint| endpoint.name.clone()).unwrap_or_default();
        let (scripts, folder, setup, id) = (scripts.to_path_buf(), folder.to_path_buf(), setup.to_vec(), cell.id.clone());
        let step = declared.step.clone();
        let cost = self.costs.line(&cell, move || {
            let pipeline = living_pipeline(&folder)?;
            let spec = match (spec, step) {
                (Some(spec), _) => spec,
                (None, Some(step)) => step_spec(&scripts, &folder, &setup, &pipeline, &step, &ItemRef { path: String::new(), item: Some(id.clone()) }).ok()?,
                (None, None) => return None,
            };
            let file = write_spec(&folder, &format!("{id}-cost"), &spec).ok()?;
            let plan = script(&scripts, &folder, &setup, "request.py", &["plan", &file, "--json"]).ok();
            let _ = std::fs::remove_file(folder.join(&file));
            let plan: Value = serde_json::from_str(&plan?).ok()?;
            Some(cost_line(&plan, &service))
        });
        CardView { cell: shown, not_canon, cost }
    }

    /// The spec a cell runs as now, its step read afresh.
    pub(crate) fn spec(&mut self, folder: &Path, scripts: &Path, setup: &[(String, String)], settings: &Settings, declared: &Cell, at: &ItemRef) -> Result<Value, String> {
        let mut spec = match &declared.step {
            Some(step) => {
                self.forget();
                let pipeline = living_pipeline(folder).ok_or("This project has no pipeline yet.")?;
                step_spec(scripts, folder, setup, &pipeline, step, at)?
            }
            None => draft_spec(settings, declared, at)?,
        };
        unique_name(folder, &mut spec);
        Ok(spec)
    }

    /// The cell's rounds, when `requests/` changed since they were last
    /// sent, or when `again` asks for them anyway.
    pub(crate) fn rounds_due(&mut self, folder: &Path, scripts: &Path, setup: &[(String, String)], cell: &Cell, again: bool) -> Option<Vec<Round>> {
        let canon = self.canon(folder, scripts, setup);
        let now = (modified(&folder.join("requests"))?, self.read_at);
        if !again && self.sent.get(&cell.id) == Some(&now) {
            return None;
        }
        self.sent.insert(cell.id.clone(), now);
        Some(rounds(folder, &cell.id, cell.step.as_ref(), &canon, &mut self.reports))
    }

    /// Saves a person's edit to a step's cell into the step, with
    /// `pipeline.py step`'s partial update: its template, references, and
    /// settings. A reference whose file is a published artifact's is kept
    /// by its artifact, as steps name canon.
    pub(crate) fn save_step(&mut self, folder: &Path, scripts: &Path, setup: &[(String, String)], step: &StepRef, input: &[CellPart], settings: Option<&Value>) -> Result<(), String> {
        let pipeline = living_pipeline(folder).ok_or("This project has no pipeline yet.")?;
        let canon = self.canon(folder, scripts, setup);
        let read = self.step(folder, scripts, setup, step).ok_or("The step could not be read.")?;
        let (template, refs, endpoint) = step_edits(&read, &canon, input, settings);
        if template.is_none() && refs.is_none() && endpoint.is_none() {
            return Ok(());
        }
        // Only what changed is sent: pipeline.py step keeps every other field.
        let mut args: Vec<&str> = vec!["step", pipeline.as_str(), step.chain.as_str(), step.step.as_str()];
        if let Some(template) = &template {
            args.extend(["--template", template.as_str()]);
        }
        if let Some(refs) = &refs {
            args.extend(["--refs", refs.as_str()]);
        }
        if let Some(endpoint) = &endpoint {
            args.extend(["--endpoint", endpoint.as_str()]);
        }
        script(scripts, folder, setup, "pipeline.py", &args)?;
        self.forget();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(parts: Vec<CellPart>) -> Cell {
        let mut cell: Cell = serde_json::from_value(json!({ "id": "c1", "kind": "image", "input": [], "state": "idle" })).unwrap();
        cell.input = parts;
        cell.endpoint = Some(String::from("openrouter"));
        cell.model = Some(String::from("openai/gpt-image-2.5-sunburst"));
        cell
    }

    /// Catches a draft run with its references out of order, without the
    /// cell named in its setup, or with a workspace file as an input.
    #[test]
    fn a_draft_becomes_its_spec() {
        let mut settings = Settings::default();
        let endpoint = settings.first_start("https://stand-in/v1").unwrap();
        let at = ItemRef { path: String::from("work/Keyframes.ipx"), item: Some(String::from("c1")) };
        let mut cell = draft(vec![
            CellPart::Text { text: String::from("The cook lifts the towel.") },
            CellPart::Path { path: String::from("library/layout.png"), media_type: String::from("image/png"), role: Some(String::from("Start frame, the layout")), field: None },
            CellPart::Path { path: String::from("library/sheet.png"), media_type: String::from("image/png"), role: Some(String::from("Reference, the cook")), field: None },
        ]);
        cell.endpoint = Some(endpoint.clone());
        let spec = draft_spec(&settings, &cell, &at).unwrap();
        assert_eq!(spec["endpoint"]["kind"], "openrouter-image");
        assert_eq!(spec["references"][0]["file"], "library/layout.png");
        assert_eq!(spec["references"][1]["role"], "Reference, the cook");
        assert_eq!(spec["setup"]["cell"]["item"], "c1");
        let mut in_work = draft(vec![CellPart::Path { path: String::from("work/board.png"), media_type: String::from("image/png"), role: None, field: None }]);
        in_work.endpoint = Some(endpoint);
        assert!(draft_spec(&settings, &in_work, &at).is_err());
    }

    /// Catches a cell's rounds read from requests it did not make, out of
    /// order, or with a take marked canon that is not.
    #[test]
    fn a_cells_rounds_come_from_its_reports() {
        let folder = std::env::temp_dir().join(format!("prompt-rounds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("requests")).unwrap();
        let report = |id: &str, started: &str, cell: &str, outputs: Value| {
            let record = json!({ "id": id, "started": started, "prompt": "p", "sent": { "model": "m" }, "outputs": outputs, "setup": { "cell": { "path": "work/a.ipx", "item": cell } } });
            std::fs::write(folder.join(format!("requests/{id}.html")), format!("<script type=\"application/json\" id=\"request\">{record}</script>")).unwrap();
        };
        report("r2", "2026-11-16T12:05:00Z", "c1", json!(["library/r2.png"]));
        report("r1", "2026-11-16T12:00:00Z", "c1", json!(["library/r1.png"]));
        report("other", "2026-11-16T12:01:00Z", "c9", json!(["library/x.png"]));
        report("failed", "2026-11-16T12:10:00Z", "c1", json!([]));
        let mut canon = Canon::default();
        canon.published.insert(String::from("library/r1.png"));
        let rounds = rounds(&folder, "c1", None, &canon, &mut Reports::default());
        let ids: Vec<&str> = rounds.iter().map(|round| round.request.as_str()).collect();
        assert_eq!(ids, ["r1", "r2", "failed"]);
        assert_eq!(rounds[0].number, 1);
        assert!(rounds[0].takes[0].canon && !rounds[1].takes[0].canon);
        assert_eq!(rounds[2].state, RoundState::Failed);
        assert_eq!(rounds[1].started, epoch("2026-11-16T12:05:00Z"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches an edit to a step's prompt that rewrites or drops its
    /// references, such as one to an artifact not made yet.
    #[test]
    fn a_step_edit_changes_only_what_changed() {
        let read = json!({
            "template": "the cook",
            "refs": r#"[{"artifact": "the diner: layout", "role": "image 1"}, {"artifact": "the cook: take not made yet", "role": "image 2"}]"#,
            "endpoint": r#"{"kind": "fal", "settings": {"seed": 7}}"#,
        });
        let mut canon = Canon::default();
        canon.files.insert(String::from("the diner: layout"), String::from("library/layout.png"));
        let shown = vec![
            CellPart::Text { text: String::from("the cook lifts the towel") },
            CellPart::Path { path: String::from("library/layout.png"), media_type: String::from("image/png"), role: Some(String::from("image 1")), field: None },
        ];
        let (template, refs, endpoint) = step_edits(&read, &canon, &shown, Some(&json!({ "seed": 7 })));
        assert_eq!(template.as_deref(), Some("the cook lifts the towel"));
        assert!(refs.is_none(), "an edit to the prompt rewrote the references: {refs:?}");
        assert!(endpoint.is_none());
        let mut moved = shown.clone();
        moved.push(CellPart::Path { path: String::from("library/new.png"), media_type: String::from("image/png"), role: None, field: None });
        let (_, refs, _) = step_edits(&read, &canon, &moved, None);
        let refs: Vec<Value> = serde_json::from_str(&refs.unwrap()).unwrap();
        assert_eq!(refs[0]["artifact"], "the diner: layout", "a published reference lost its artifact");
        assert_eq!(refs[1]["file"], "library/new.png");
        assert_eq!(refs[2]["artifact"], "the cook: take not made yet", "a reference with no file yet was dropped");
    }

    #[test]
    fn a_records_time_reads_as_seconds() {
        assert_eq!(epoch("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch("2026-11-16T12:00:00Z"), Some(1_794_830_400));
    }

    /// Catches a reference called canon whose file is not published.
    #[test]
    fn a_reference_is_canon_only_when_published() {
        let cell = draft(vec![
            CellPart::Text { text: String::from("p") },
            CellPart::Path { path: String::from("library/a.png"), media_type: String::from("image/png"), role: None, field: None },
            CellPart::Path { path: String::from("library/b.png"), media_type: String::from("image/png"), role: None, field: None },
        ]);
        let mut canon = Canon::default();
        canon.published.insert(String::from("library/a.png"));
        assert_eq!(not_canon(&cell, &canon), [1]);
    }

    #[test]
    fn the_cost_line_says_unknown_never_zero() {
        assert_eq!(cost_line(&json!({ "estimate": "about $0.24 a request" }), "OpenRouter"), "About $0.24 a request · OpenRouter");
        assert_eq!(cost_line(&json!({ "estimate": "unknown: no price for this endpoint" }), "fal.ai"), "Cost unknown · fal.ai");
    }

    #[test]
    fn a_recorded_step_names_its_request() {
        let argv: Vec<String> = ["python3", "/s/pipeline.py", "step", "002", "the line", "keyframe", "--request", "r7"].iter().map(|s| s.to_string()).collect();
        let (step, request) = recorded_step(&argv).unwrap();
        assert_eq!((step.chain.as_str(), step.step.as_str(), request.as_str()), ("the line", "keyframe", "r7"));
    }
}
