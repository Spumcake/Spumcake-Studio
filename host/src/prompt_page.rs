//! The Prompt page's runner duties: what the card
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
//! - A cell's rounds are read from `requests/` and the pending records, and
//!   sent to its workspace with `takes`.

use protocol::{Cell, CellPart, ItemRef, Round, RoundState, SentReference, StepRef, TakeFile};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use crate::workspace::{EndpointKind, Settings};

/// The skill whose every request finishes a take, as Spumcake-Skills'
/// reports record it.
const FINISHING_SKILL: &str = "generate-finished-shot";
/// The skill of Last order's older finishing rounds, which
/// generate-finished-shot claims as its own: one of its requests finishes
/// a take when it sends exactly one file in `FINISHING_FIELD`.
const OLDER_FINISHING_SKILL: &str = "create-final-animation";
/// The field Last order's older finishing rounds sent their take in.
const FINISHING_FIELD: &str = "video_url";

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
    /// The artifact each step makes, by its chain and its name.
    pub step_artifacts: HashMap<(String, String), String>,
    /// The skill each step follows, by its chain and its name.
    pub step_skills: HashMap<(String, String), String>,
    /// Each step's place in its chain, by its chain and its name: the
    /// steps at one place are one cell's assets in use.
    pub step_orders: HashMap<(String, String), i64>,
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
pub(crate) fn script(scripts: &Path, folder: &Path, setup: &[(String, String)], name: &str, args: &[&str]) -> Result<String, String> {
    // Never a key: Studio's own process may hold one from its `.env`.
    let out = crate::runner::strip_keys(&mut Command::new("python3"))
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
    // Each step's artifact as one JSON row, since a name can hold " | ".
    let steps = script(scripts, folder, setup, "pipeline.py", &["sql", pipeline, "SELECT json_array(chain, name, artifact) FROM step WHERE artifact IS NOT NULL AND artifact != ''", "--read-only"]).unwrap_or_default();
    for row in steps.lines() {
        if let Ok(Value::Array(cells)) = serde_json::from_str::<Value>(row)
            && let [Value::String(chain), Value::String(step), Value::String(artifact)] = cells.as_slice()
        {
            canon.step_artifacts.insert((chain.clone(), step.clone()), artifact.clone());
        }
    }
    let places = script(scripts, folder, setup, "pipeline.py", &["sql", pipeline, "SELECT json_array(chain, name, ord, skill) FROM step", "--read-only"]).unwrap_or_default();
    for row in places.lines() {
        if let Ok(Value::Array(cells)) = serde_json::from_str::<Value>(row)
            && let [Value::String(chain), Value::String(step), order, skill] = cells.as_slice()
        {
            let key = (chain.clone(), step.clone());
            if let Some(order) = order.as_i64() {
                canon.step_orders.insert(key.clone(), order);
            }
            if let Some(skill) = skill.as_str().filter(|skill| !skill.is_empty()) {
                canon.step_skills.insert(key, skill.to_string());
            }
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
        Some("flac") => "audio/flac",
        Some("aif" | "aiff") => "audio/aiff",
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

/// The user's endpoint for a model whose cell names none, as a workspace
/// cannot know the user's endpoints: the one whose kind is the service the
/// model's workflow skill names for it in its `models.json`.
pub(crate) fn endpoint_for(settings: &Settings, skills: &[crate::skills_setup::Skill], model: &str, skill: Option<&str>) -> Option<String> {
    // The step's skill's own route first, its page's "Runs on", which the
    // user chose: a stand-in or an OpenAI-compatible address of their own.
    if let Some(route) = skill.and_then(|skill| settings.skill_routes.get(skill))
        && settings.endpoint(&route.endpoint).is_some()
    {
        return Some(route.endpoint.clone());
    }
    let service = crate::prices::local_service(skills, model)?.to_lowercase();
    settings
        .endpoints
        .iter()
        .find(|endpoint| match (service.as_str(), &endpoint.kind) {
            ("fal", EndpointKind::Fal) | ("openrouter", EndpointKind::Openrouter) => true,
            ("openai", EndpointKind::Openai) => endpoint.address.contains("openai.com"),
            _ => false,
        })
        .map(|endpoint| endpoint.id.clone())
}

/// The endpoint a cell runs on: its own, or else its step's skill's route,
/// or else the one its model's service names.
pub(crate) fn cell_endpoint(settings: &Settings, cell: &Cell, skill: Option<&str>) -> Option<String> {
    cell.endpoint.clone().or_else(|| {
        let skills = crate::skills_setup::listed_skills(&crate::skills_checkout::skill_folders(&settings.skill_folders));
        endpoint_for(settings, &skills, cell.model.as_deref().unwrap_or(""), skill)
    })
}

/// Why a sound cell does not run on an endpoint other than fal.ai.
const SOUND_ON_FAL: &str = "Sound is made only through fal.ai.";

/// The kind of request `request.py` makes for a cell on this endpoint.
fn request_kind(settings: &Settings, cell: &Cell) -> Result<&'static str, String> {
    let endpoint = cell_endpoint(settings, cell, None).and_then(|id| settings.endpoint(&id).cloned()).ok_or("This cell has no endpoint. Choose one for it.")?;
    let image = cell.kind == protocol::IMAGE_KIND;
    let video = cell.kind == protocol::VIDEO_KIND;
    let sound = cell.kind == protocol::SOUND_KIND;
    let has_references = !references(cell).is_empty();
    Ok(match endpoint.kind {
        EndpointKind::Fal => "fal",
        // A voice take or a sound bed is made through fal, as Last order's
        // were; anywhere else it would be sent as text.
        _ if sound => return Err(String::from(SOUND_ON_FAL)),
        EndpointKind::Openrouter if image => "openrouter-image",
        EndpointKind::Openrouter if !video => "openrouter-text",
        EndpointKind::Openai if endpoint.address.contains("openai.com") && image => {
            if has_references { "openai-image-edit" } else { "openai-image" }
        }
        EndpointKind::Openai if endpoint.address.contains("openai.com") && !video => "openai-text",
        _ => return Err(format!("This cell cannot run on “{}”: Spumcake-Skills sends requests only to OpenRouter, OpenAI, and fal.", endpoint.name)),
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

/// Writes a step's template into Studio's scratch files, for a template
/// `pipeline.py` would otherwise read as a file's name. Returns its path,
/// relative to the project.
pub(crate) fn write_template(folder: &Path, template: &str) -> Result<String, String> {
    let dir = folder.join(protocol::PROJECT_DIR).join("cache").join("templates");
    std::fs::create_dir_all(&dir).map_err(|error| format!("Could not make {}: {error}", dir.display()))?;
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|elapsed| elapsed.as_millis()).unwrap_or(0);
    let name = format!("template-{stamp}.txt");
    std::fs::write(dir.join(&name), template).map_err(|error| format!("Could not write the template: {error}"))?;
    Ok(format!("{}/cache/templates/{name}", protocol::PROJECT_DIR))
}

/// The cost line under the card, from a plan's estimate and the service.
pub(crate) fn cost_line(plan: &Value, service: &str) -> String {
    let estimate = plan.get("estimate").and_then(Value::as_str).unwrap_or("");
    let shown = match (estimate.strip_prefix("about "), estimate.strip_prefix("up to about ")) {
        (Some(rest), _) => format!("About {rest}"),
        (_, Some(rest)) => format!("Up to about {rest}"),
        _ => String::from("Cost unknown"),
    };
    if service.is_empty() { shown } else { format!("{shown} · {service}") }
}

/// Seconds since the Unix epoch, from a record's `2026-11-14T13:30:00Z`.
pub(crate) fn epoch(time: &str) -> Option<u64> {
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
/// the cell in its `setup`, or its step, or whose output is the cell's
/// file, with the rounds that file was finished from; and each pending
/// record, oldest first, numbered from 1.
/// What `rounds` keeps of each report, by its file and time, so a report
/// is read once, not on every change to `requests/`.
pub(crate) type Reports = HashMap<PathBuf, (Option<SystemTime>, Option<Value>)>;

/// A report's record, cut to what a round needs.
fn slim(record: Value) -> Value {
    let keep = |key: &str| record.get(key).cloned().unwrap_or(Value::Null);
    let references: Vec<Value> = record
        .get("references")
        .and_then(Value::as_array)
        .map(|references| references.iter().map(|reference| json!({ "file": reference.get("file"), "role": reference.get("role"), "field": reference.get("field") })).collect())
        .unwrap_or_default();
    json!({
        "setup": keep("setup"), "chain": keep("chain"), "step": keep("step"), "skill": keep("skill"), "started": keep("started"),
        "outputs": keep("outputs"), "prompt": keep("prompt"), "references": references,
        "sent": { "model": record.pointer("/sent/model").cloned().unwrap_or(Value::Null) },
        "endpoint": { "model": record.pointer("/endpoint/model").cloned().unwrap_or(Value::Null) },
    })
}

pub(crate) fn rounds(folder: &Path, cell: &str, step: Option<&StepRef>, outputs: &[String], canon: &Canon, reports: &mut Reports) -> Vec<Round> {
    let mut found: Vec<(u64, Round)> = Vec::new();
    // A step's cell is every step at its place in the chain, one per asset
    // in use, such as "final shot" and "final shot (2)"; a step with no place
    // known is its cell alone.
    let order = step.and_then(|step| canon.step_orders.get(&(step.chain.clone(), step.step.clone())));
    let cell_steps: Vec<String> = match (step, order) {
        (Some(step), Some(order)) => canon.step_orders.iter().filter(|((chain, _), at)| *chain == step.chain && *at == order).map(|((_, name), _)| name.clone()).collect(),
        (Some(step), None) => vec![step.step.clone()],
        (None, _) => Vec::new(),
    };
    // A report that names the step's chain and no step is a take made for
    // the chain, as Last order's LTX trial of its final shot is: it is the
    // cell's when it follows the step's skill and no other place in the
    // chain has a step that follows that skill.
    let chain_take = |record: &Value, step: &StepRef| {
        let Some(skill) = canon.step_skills.get(&(step.chain.clone(), step.step.clone())) else { return false };
        // A step whose place is not known is a place of its own.
        let places: HashSet<String> = canon.step_skills.iter().filter(|((chain, _), other)| *chain == step.chain && *other == skill).map(|(key, _)| canon.step_orders.get(key).map_or_else(|| format!("step {}", key.1), |order| order.to_string())).collect();
        record.get("step").and_then(Value::as_str).is_none() && record.get("skill").and_then(Value::as_str) == Some(skill.as_str()) && places.len() == 1
    };
    let names_cell = |record: &Value| {
        record.pointer("/setup/cell/item").and_then(Value::as_str) == Some(cell)
            || step.is_some_and(|step| {
                record.get("chain").and_then(Value::as_str) == Some(step.chain.as_str())
                    && (record.get("step").and_then(Value::as_str).is_some_and(|named| cell_steps.iter().any(|own| own == named)) || chain_take(record, step))
            })
    };
    // Each report, by its id, with its record and the file it is in.
    let mut done: Vec<(String, String, Value)> = Vec::new();
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
            done.push((id.to_string(), name, record));
        }
    }
    let outputs_of = |record: &Value| -> Vec<String> { record.get("outputs").and_then(Value::as_array).map(|outputs| outputs.iter().filter_map(|output| output.as_str().map(String::from)).collect()).unwrap_or_default() };
    // The cell's reports: those that name it, first, then those whose
    // output is the cell's file, which a report that names neither cell nor
    // step, as Last order's first pipeline's do, is found by: the cell's
    // own outputs, and the file its step's artifact is published as.
    let mut matched: Vec<bool> = done.iter().map(|(_, _, record)| names_cell(record)).collect();
    let mut seeds: HashSet<String> = outputs.iter().cloned().collect();
    if let Some(step) = step {
        for own in &cell_steps {
            if let Some(file) = canon.step_artifacts.get(&(step.chain.clone(), own.clone())).and_then(|artifact| canon.files.get(artifact)) {
                seeds.insert(file.clone());
            }
        }
    }
    // Then the rounds a found take was finished through, as a retime or an
    // upscale is: a request of the finishing skill, whose one reference of
    // its own media is the take it finishes, or one of Last order's older
    // finishing rounds, whose one reference in the finishing field is. Each found take brings what was finished from it, and each found
    // round the take it finished, back to the round that made it; never the
    // other rounds made from that one, nor a take made from another as a
    // reference, such as an image from another image, and never a report
    // that names another step or another cell.
    let finished_from = |record: &Value| -> Option<String> {
        let references = record.get("references").and_then(Value::as_array).into_iter().flatten();
        let finished: Vec<&str> = match record.get("skill").and_then(Value::as_str) {
            Some(FINISHING_SKILL) => {
                let media = media_type(outputs_of(record).first()?);
                references.filter_map(|reference| reference.get("file").and_then(Value::as_str)).filter(|path| media_type(path) == media).collect()
            }
            Some(OLDER_FINISHING_SKILL) => references.filter(|reference| reference.get("field").and_then(Value::as_str) == Some(FINISHING_FIELD)).filter_map(|reference| reference.get("file").and_then(Value::as_str)).collect(),
            _ => return None,
        };
        if let [only] = finished.as_slice() { Some(only.to_string()) } else { None }
    };
    let own_step = |record: &Value| {
        let named = |key: &str, own: Option<&str>| match record.get(key).and_then(Value::as_str) {
            None => true,
            named => named == own,
        };
        let other_cell = record.pointer("/setup/cell/item").and_then(Value::as_str).is_some_and(|item| item != cell);
        let own_step = match record.get("step").and_then(Value::as_str) {
            None => true,
            Some(named) => cell_steps.iter().any(|own| own == named),
        };
        !other_cell && own_step && (step.is_none() || named("chain", step.map(|step| step.chain.as_str())))
    };
    for (index, (_, _, record)) in done.iter().enumerate() {
        if outputs_of(record).iter().any(|output| seeds.contains(output)) {
            matched[index] = true;
        }
    }
    let first: Vec<bool> = matched.clone();
    // Forward, from the found rounds' takes to what was finished from them.
    let mut after = first.clone();
    loop {
        let takes: HashSet<String> = done.iter().zip(&after).filter(|(_, found)| **found).flat_map(|((_, _, record), _)| outputs_of(record)).collect();
        let mut grew = false;
        for (index, (_, _, record)) in done.iter().enumerate() {
            if !after[index] && own_step(record) && finished_from(record).is_some_and(|source| takes.contains(&source)) {
                after[index] = true;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    // Back, from the found rounds to the takes they finished.
    let mut before = first;
    loop {
        let sources: HashSet<String> = done.iter().zip(&before).filter(|(_, found)| **found).filter_map(|((_, _, record), _)| finished_from(record)).collect();
        let mut grew = false;
        for (index, (_, _, record)) in done.iter().enumerate() {
            if !before[index] && own_step(record) && outputs_of(record).iter().any(|output| sources.contains(output)) {
                before[index] = true;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let matched: Vec<bool> = after.iter().zip(&before).map(|(after, before)| *after || *before).collect();
    for ((id, name, record), matched) in done.iter().zip(matched) {
        if !matched {
            continue;
        }
        let started = record.get("started").and_then(Value::as_str).and_then(epoch).unwrap_or(0);
        let outputs = outputs_of(record);
        let failed = outputs.is_empty();
        let report = format!("requests/{name}");
        let state = if failed { RoundState::Failed } else { RoundState::Done };
        let error = failed.then(|| String::from("Nothing came back. Open its report to see why."));
        let mut round = round_of(id, record, state, started, &outputs, error, Some(report));
        // The canon artifact of the step that made it, one of the cell's
        // steps, or else the cell's own step's.
        round.artifact = step.and_then(|step| {
            let own = record.get("step").and_then(Value::as_str).filter(|named| cell_steps.iter().any(|own| own == named)).unwrap_or(step.step.as_str());
            canon.step_artifacts.get(&(step.chain.clone(), own.to_string())).cloned()
        });
        found.push((started, round));
    }
    found.sort_by_key(|(started, round)| (*started, round.request.clone()));
    found
        .into_iter()
        .enumerate()
        .map(|(index, (_, mut round))| {
            round.number = index as u32 + 1;
            // A pending round is its cell's own step's.
            if round.artifact.is_none() {
                round.artifact = step.and_then(|step| canon.step_artifacts.get(&(step.chain.clone(), step.step.clone())).cloned());
            }
            for take in &mut round.takes {
                take.canon = canon.published.contains(&take.path);
            }
            round
        })
        .collect()
}

/// Each take of the rounds as its asset stands: its iterations, the canon
/// mark while it is in use with its artifact, and its outline or Unverified
/// mark. A take that is no asset's first, such as an iteration made by a
/// fix, keeps what its round gave it.
pub(crate) fn fill_takes(rounds: &mut [Round], assets: &[crate::assets::Asset]) {
    for take in rounds.iter_mut().flat_map(|round| round.takes.iter_mut()) {
        let Some(asset) = assets.iter().find(|asset| asset.first == take.path) else { continue };
        take.iterations = asset.iterations.clone();
        take.canon = asset.in_use;
        take.artifact = asset.artifact.clone().filter(|_| asset.in_use);
        take.outlined = asset.outlined;
        take.unverified = asset.unverified;
    }
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
        model_name: String::new(),
        prompt: record.get("prompt").and_then(Value::as_str).unwrap_or("").to_string(),
        references,
        left_out: Vec::new(),
        state,
        started: (started > 0).then_some(started),
        count: None,
        takes: outputs.iter().map(|path| TakeFile { path: path.clone(), media_type: media_type(path).to_string(), ..Default::default() }).collect(),
        error,
        report,
        artifact: None,
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
        // A reference with no file to show stays as it was, in its place:
        // after as many shown references as came before it.
        let mut hidden: Vec<(usize, &Value)> = Vec::new();
        let mut before = 0;
        for reference in &old {
            if file_of(reference).is_none() {
                hidden.push((before, reference));
            } else {
                before += 1;
            }
        }
        let mut refs: Vec<Value> = Vec::new();
        for (at, (path, role, field)) in edited.iter().enumerate() {
            refs.extend(hidden.iter().filter(|(place, _)| *place == at).map(|(_, reference)| (*reference).clone()));
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
            match field {
                Some(field) => reference["field"] = json!(field),
                None => {
                    if let Some(fields) = reference.as_object_mut() {
                        fields.remove("field");
                    }
                }
            }
            refs.push(reference);
        }
        refs.extend(hidden.iter().filter(|(place, _)| *place >= edited.len()).map(|(_, reference)| (*reference).clone()));
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
    pub cost: Option<String>,
}

/// The Prompt page's runner state for the open project: canon and steps as
/// last read, held here only, and each cell's cost and rounds sent.
#[derive(Default)]
pub(crate) struct PromptPage {
    /// `canon.sql`'s time when canon and the steps were read.
    read_at: Option<SystemTime>,
    /// Canon was read at `read_at`; a pipeline with no `canon.sql` yet is
    /// read once, not every frame.
    read: bool,
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
        if self.read && now == self.read_at {
            return;
        }
        self.read_at = now;
        self.read = true;
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
        self.read = false;
    }

    /// What the card shows of a declared cell.
    pub(crate) fn card(&mut self, folder: &Path, scripts: &Path, setup: &[(String, String)], settings: &Settings, declared: &Cell, at: &ItemRef) -> CardView {
        let read = declared.step.as_ref().and_then(|step| self.step(folder, scripts, setup, step));
        let skill = read.as_ref().and_then(|read| read.get("skill").and_then(Value::as_str).map(String::from));
        let shown = read.as_ref().map(|read| step_cell(declared, read, &self.canon));
        self.refresh(folder, scripts, setup);
        let cell = shown.clone().unwrap_or_else(|| declared.clone());
        // The cost is worked out from the spec the run would send.
        let spec = match &declared.step {
            Some(_) => None,
            None => draft_spec(settings, &cell, at).ok(),
        };
        let service = cell_endpoint(settings, &cell, skill.as_deref()).and_then(|id| settings.endpoint(&id).map(|endpoint| endpoint.name.clone())).unwrap_or_default();
        let (scripts, folder, setup, id) = (scripts.to_path_buf(), folder.to_path_buf(), setup.to_vec(), cell.id.clone());
        let step = declared.step.clone();
        let cost = self.costs.line(&cell, move || {
            let pipeline = living_pipeline(&folder)?;
            let spec = match (spec, step) {
                (Some(spec), _) => spec,
                (None, Some(step)) => step_spec(&scripts, &folder, &setup, &pipeline, &step, &ItemRef { path: String::new(), item: Some(id.clone()) }).ok()?,
                (None, None) => return None,
            };
            // The model's price, read from its service at most once a day,
            // goes into the setup file before the plan is made.
            if let Some(data) = crate::prices::data_of(&setup)
                && crate::prices::ensure(&data, &spec, &setup)
            {
                crate::runner::write_setup(&data);
            }
            let file = write_spec(&folder, &format!("{id}-cost"), &spec).ok()?;
            let plan = script(&scripts, &folder, &setup, "request.py", &["plan", &file, "--json"]).ok();
            let _ = std::fs::remove_file(folder.join(&file));
            let mut plan: Value = serde_json::from_str(&plan?).ok()?;
            if let Some(data) = crate::prices::data_of(&setup) {
                crate::prices::fill_from_token_prices(&data, &folder, &mut plan);
            }
            crate::prices::fill_from_history(&folder, &mut plan);
            Some(cost_line(&plan, &service))
        });
        CardView { cell: shown, cost }
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
        let outputs: Vec<String> = cell.output.iter().filter_map(|part| match part {
            CellPart::Path { path, .. } => Some(path.clone()),
            _ => None,
        }).collect();
        let mut found = rounds(folder, &cell.id, cell.step.as_ref(), &outputs, &canon, &mut self.reports);
        // Each take as its asset stands: its iterations, whether it is in
        // use, and whether Skills finds it out of date.
        if let Some(pipeline) = living_pipeline(folder) {
            let touches = script(scripts, folder, setup, "pipeline.py", &["touches", &pipeline, "--json"]).ok().and_then(|printed| serde_json::from_str(&printed).ok()).unwrap_or(Value::Null);
            fill_takes(&mut found, &crate::assets::assets(folder, &canon, &touches));
        }
        Some(found)
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
        // pipeline.py reads a template that starts with "@" as a file's name,
        // so such a prompt is given as a file holding it.
        let template = match template {
            Some(template) if template.starts_with('@') => Some(format!("@{}", write_template(folder, &template)?)),
            template => template,
        };
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

    /// Generate again on a step's cell: a new step at the cell's place,
    /// `"<step> (N)"`, holding the cell's prompt, references, and settings as
    /// they stand, with its own artifact, `"<element>: <role> (N)"`, as
    /// Skills' project-pipeline names a second alternative. Returns the new
    /// step, which the runner then makes a take of with `rerun --step`.
    pub(crate) fn generate_again(&mut self, folder: &Path, scripts: &Path, setup: &[(String, String)], step: &StepRef, input: &[CellPart], settings: Option<&Value>) -> Result<StepRef, String> {
        let pipeline = living_pipeline(folder).ok_or("This project has no pipeline yet.")?;
        let canon = self.canon(folder, scripts, setup);
        let read = self.step(folder, scripts, setup, step).ok_or("The step could not be read.")?;
        let order = read.get("ord").and_then(Value::as_i64).ok_or("The step has no place in its chain.")?;
        let base = alternative_of(&step.step).0;
        let taken = |name: &str| canon.step_orders.contains_key(&(step.chain.clone(), name.to_string()));
        let n = (2..).find(|n| !taken(&alternative_name(base, *n))).unwrap_or(2);
        let name = alternative_name(base, n);
        let artifact = canon.step_artifacts.get(&(step.chain.clone(), step.step.clone())).map(|artifact| alternative_name(alternative_of(artifact).0, n));
        let (template, refs, endpoint) = step_edits(&read, &canon, input, settings);
        let text = |key: &str| read.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        let template = template.unwrap_or_else(|| text("template"));
        // pipeline.py reads a template that starts with "@" as a file's name.
        let template = if template.starts_with('@') { format!("@{}", write_template(folder, &template)?) } else { template };
        let refs = refs.unwrap_or_else(|| text("refs"));
        let endpoint = endpoint.unwrap_or_else(|| text("endpoint"));
        let (skill, order) = (text("skill"), order.to_string());
        let mut args: Vec<&str> = vec!["step", pipeline.as_str(), step.chain.as_str(), name.as_str(), "--order", order.as_str()];
        for (flag, value) in [("--skill", &skill), ("--endpoint", &endpoint), ("--template", &template), ("--refs", &refs)] {
            if !value.is_empty() {
                args.extend([flag, value.as_str()]);
            }
        }
        if let Some(artifact) = &artifact {
            args.extend(["--artifact", artifact.as_str()]);
        }
        script(scripts, folder, setup, "pipeline.py", &args)?;
        self.forget();
        Ok(StepRef { chain: step.chain.clone(), step: name })
    }
}

/// A step's or an artifact's name without its alternative's number, and
/// the number, as Skills' project-pipeline names an alternative:
/// `"keyframe, wiping (2)"` is `("keyframe, wiping", Some(2))`.
pub(crate) fn alternative_of(name: &str) -> (&str, Option<u32>) {
    let Some((base, number)) = name.strip_suffix(')').and_then(|rest| rest.rsplit_once(" (")) else { return (name, None) };
    match number.parse::<u32>() {
        Ok(number) if !base.is_empty() => (base, Some(number)),
        _ => (name, None),
    }
}

/// The name of an alternative, the first's with its number in brackets:
/// `"keyframe, wiping (2)"`, its artifact `"the diner: keyframe, wiping (2)"`.
pub(crate) fn alternative_name(base: &str, number: u32) -> String {
    format!("{base} ({number})")
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

    /// Catches a Retime or Upscale cell, which a workspace makes with no
    /// endpoint, refused when its model's skill names fal and the user has
    /// a fal endpoint.
    #[test]
    fn a_cell_with_no_endpoint_takes_its_models_service() {
        let folder = std::env::temp_dir().join(format!("prompt-service-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("models.json"), json!([{ "service": "fal", "model": "topaz/upscale/video/precision", "name": "Topaz Video Upscale" }]).to_string()).unwrap();
        let skill = crate::skills_setup::Skill { name: String::from("finish-shot"), description: String::new(), implements: Vec::new(), helps: Vec::new(), folder: folder.clone(), root: folder.clone() };
        let mut settings = Settings::default();
        settings.endpoints.push(crate::workspace::EndpointSetting { id: String::from("fal-1"), name: String::from("fal.ai"), kind: EndpointKind::Fal, address: String::from("https://fal.ai"), description: String::new() });
        assert_eq!(endpoint_for(&settings, std::slice::from_ref(&skill), "topaz/upscale/video/precision", None).as_deref(), Some("fal-1"));
        assert_eq!(endpoint_for(&settings, std::slice::from_ref(&skill), "other/model", None), None, "a model no skill names took an endpoint");
        settings.endpoints.clear();
        assert_eq!(endpoint_for(&settings, std::slice::from_ref(&skill), "topaz/upscale/video/precision", None), None, "a service the user has no endpoint for took one");
        // A step's skill's route, an OpenAI-compatible stand-in the user
        // chose on its page, comes before the address match.
        settings.endpoints.push(crate::workspace::EndpointSetting { id: String::from("images"), name: String::from("Images"), kind: EndpointKind::Openai, address: String::from("http://127.0.0.1:9"), description: String::new() });
        assert_eq!(endpoint_for(&settings, &[skill.clone()], "gpt-image-2.6-sunburst", Some("generate-keyframe")), None, "an address not OpenAI's was taken with no route");
        settings.skill_routes.insert(String::from("generate-keyframe"), crate::workspace::SkillRoute { endpoint: String::from("images"), model: String::from("gpt-image-2.6-sunburst") });
        assert_eq!(endpoint_for(&settings, &[skill], "gpt-image-2.6-sunburst", Some("generate-keyframe")).as_deref(), Some("images"), "the step's skill's route was passed over");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a voice take or a sound bed sent anywhere but fal, where it
    /// would go as text, or its takes read as files of unknown kind.
    #[test]
    fn a_sound_cell_runs_through_fal() {
        let mut settings = Settings::default();
        let openrouter = settings.first_start("https://stand-in/v1").unwrap();
        settings.endpoints.push(crate::workspace::EndpointSetting { id: String::from("fal-1"), name: String::from("fal.ai"), kind: EndpointKind::Fal, address: String::from("https://fal.ai"), description: String::new() });
        let at = ItemRef { path: String::from("work/Sound.sbx"), item: Some(String::from("c1")) };
        let mut cell = draft(vec![CellPart::Text { text: String::from("The diner at closing: the fridge hums, a tap drips.") }]);
        cell.kind = protocol::SOUND_KIND.to_string();
        cell.model = Some(String::from("bytedance/seed-audio-1.0"));
        cell.settings = Some(json!({ "output_format": "wav", "sample_rate": 48000 }));
        cell.endpoint = Some(String::from("fal-1"));
        let spec = draft_spec(&settings, &cell, &at).unwrap();
        assert_eq!(spec["endpoint"]["kind"], "fal");
        assert_eq!(spec["endpoint"]["model"], "bytedance/seed-audio-1.0");
        assert_eq!(spec["endpoint"]["settings"]["sample_rate"], 48000);
        assert_eq!(spec["setup"]["cell"]["item"], "c1");
        cell.endpoint = Some(openrouter);
        assert_eq!(draft_spec(&settings, &cell, &at).unwrap_err(), SOUND_ON_FAL, "a sound cell was sent to OpenRouter");
        for (file, kind) in [("a.wav", "audio/wav"), ("a.mp3", "audio/mpeg"), ("a.flac", "audio/flac"), ("a.aiff", "audio/aiff")] {
            assert_eq!(media_type(file), kind);
        }
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
        let rounds = rounds(&folder, "c1", None, &[], &canon, &mut Reports::default());
        let ids: Vec<&str> = rounds.iter().map(|round| round.request.as_str()).collect();
        assert_eq!(ids, ["r1", "r2", "failed"]);
        assert_eq!(rounds[0].number, 1);
        assert!(rounds[0].takes[0].canon && !rounds[1].takes[0].canon);
        assert_eq!(rounds[2].state, RoundState::Failed);
        assert_eq!(rounds[1].started, epoch("2026-11-16T12:05:00Z"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a report that names neither cell nor step, as Last order's
    /// first pipeline's do, left out of the cell whose file it made, and the
    /// retime and upscale made from that take, each with its own role, as
    /// Last order's are; and catches the images a take was made from,
    /// another step's video, or another take made from the same motion
    /// reference, or the other images made from the same one, pulled in as
    /// its rounds.
    #[test]
    fn a_report_with_no_step_opens_with_the_file_it_made() {
        let folder = std::env::temp_dir().join(format!("prompt-rounds-by-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("requests")).unwrap();
        let report = |id: &str, started: &str, step: Value, skill: &str, outputs: Value, references: Value| {
            let record = json!({ "id": id, "started": started, "prompt": "p", "sent": { "model": "m" }, "outputs": outputs, "references": references, "chain": step.get("chain"), "step": step.get("step"), "skill": skill, "setup": null });
            std::fs::write(folder.join(format!("requests/{id}.html")), format!("<script type=\"application/json\" id=\"request\">{record}</script>")).unwrap();
        };
        let reference = |file: &str, role: &str| json!({ "file": file, "role": role });
        // A finishing request sends its take in the finishing field.
        let finishes = |file: &str, role: &str| json!({ "file": file, "role": role, "field": FINISHING_FIELD });
        let none = json!({});
        let chain = json!({ "chain": "the final shot" });
        report("sheet", "2026-11-14T10:00:00Z", none.clone(), "generate-model-sheet", json!(["library/sheet.png"]), json!([]));
        report("line art", "2026-11-14T10:30:00Z", json!({ "chain": "the line-art shot", "step": "line-art shot" }), "create-line-animation", json!(["library/line-art.mp4"]), json!([]));
        report("keyframe", "2026-11-14T11:00:00Z", none.clone(), "generate-keyframe", json!(["library/keyframe.png"]), json!([reference("library/layout.png", "image 1"), reference("library/sheet.png", "image 2")]));
        report("shot", "2026-11-14T12:00:00Z", none.clone(), "create-final-animation", json!(["library/shot.mp4"]), json!([reference("library/keyframe.png", "start"), reference("library/line-art.mp4", "Video 1, the motion")]));
        // Last order's older retime: its role is its own, its field is the mark.
        report("retime", "2026-11-14T13:00:00Z", chain.clone(), "create-final-animation", json!(["library/shot-60.mp4"]), json!([finishes("library/shot.mp4", "the approved final shot, at 24 fps")]));
        // generate-finished-shot's skill alone marks it, whatever field its model takes.
        report("upscale", "2026-11-14T14:00:00Z", chain.clone(), "generate-finished-shot", json!(["library/shot-4k.mp4"]), json!([reference("library/shot-60.mp4", "the final shot at 60 fps, 1344x768")]));
        report("other", "2026-11-14T15:00:00Z", none.clone(), "", json!(["library/other.mp4"]), json!([reference("library/shot.mp4", "Video 1"), reference("library/elsewhere.mp4", "Video 2")]));
        report("sibling", "2026-11-14T16:00:00Z", none.clone(), "create-final-animation", json!(["library/sibling.mp4"]), json!([finishes("library/line-art.mp4", "the control video")]));
        // A take made for the chain with the final shot's skill, as the LTX
        // trial is, and a start frame made for it with the keyframes' skill,
        // which two of the chain's steps follow.
        report("trial", "2026-11-14T12:30:00Z", chain.clone(), "create-final-animation", json!(["library/trial.mp4"]), json!([finishes("library/previs.mp4", "the control video")]));
        report("start frame", "2026-11-14T11:30:00Z", chain.clone(), "generate-keyframe", json!(["library/start.png"]), json!([reference("library/previs.png", "image 1")]));
        // Images made each from one image alone, as Last order's first
        // pipeline made its studies from its texture.
        report("texture", "2026-11-11T09:00:00Z", none.clone(), "", json!(["library/texture.png"]), json!([]));
        report("otis", "2026-11-11T09:30:00Z", none.clone(), "generate-canonical-image", json!(["library/otis.png"]), json!([reference("library/texture.png", "image 1")]));
        report("dusk", "2026-11-11T10:00:00Z", none.clone(), "generate-canonical-image", json!(["library/dusk.png"]), json!([reference("library/texture.png", "image 1")]));
        let step = StepRef { chain: String::from("the final shot"), step: String::from("final shot") };
        let mut canon = Canon::default();
        canon.step_artifacts.insert((step.chain.clone(), step.step.clone()), String::from("the final shot: final shot"));
        canon.files.insert(String::from("the final shot: final shot"), String::from("library/shot.mp4"));
        for (name, skill) in [("final shot", "create-final-animation"), ("final frame, waiting", "generate-keyframe"), ("final frame, wiping", "generate-keyframe")] {
            canon.step_skills.insert((step.chain.clone(), name.to_string()), skill.to_string());
        }
        let ids = |rounds: Vec<Round>| rounds.iter().map(|round| round.request.clone()).collect::<Vec<_>>();
        // The step's published take, the chain's trial, then what was finished.
        let shot = rounds(&folder, "final", Some(&step), &[], &canon, &mut Reports::default());
        assert_eq!(ids(shot), ["shot", "trial", "retime", "upscale"], "the final shot's rounds, with its trial and its finishing and nothing else");
        // The start frame is neither keyframe step's alone, so it is neither's.
        let waiting = StepRef { chain: step.chain.clone(), step: String::from("final frame, waiting") };
        assert!(rounds(&folder, "waiting", Some(&waiting), &[], &canon, &mut Reports::default()).is_empty(), "a take two steps could own was given to one");
        // A cell whose file is the finished take, back to its generation.
        let finished = rounds(&folder, "finished", None, &[String::from("library/shot-4k.mp4")], &Canon::default(), &mut Reports::default());
        assert_eq!(ids(finished), ["shot", "retime", "upscale"], "a finished take's rounds, back to its generation and no further");
        // An image whose cell names its take, made from two images.
        let image = rounds(&folder, "mood", None, &[String::from("library/keyframe.png")], &Canon::default(), &mut Reports::default());
        assert_eq!(ids(image), ["keyframe"], "a take made from several images pulled them in as its rounds");
        let dusk = rounds(&folder, "dusk", None, &[String::from("library/dusk.png")], &Canon::default(), &mut Reports::default());
        assert_eq!(ids(dusk), ["dusk"], "an image made from another pulled in that one's family as its rounds");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a take shown without its iterations, with the canon mark
    /// while not in use, or with an outline on an asset not in use.
    #[test]
    fn a_take_shows_its_asset() {
        let take = |path: &str| protocol::TakeFile { path: path.into(), media_type: "image/png".into(), canon: true, iterations: Vec::new(), outlined: false, unverified: false, artifact: None };
        let mut round: Round = serde_json::from_value(json!({ "request": "bell", "number": 1, "model": "m", "state": "done" })).unwrap();
        round.takes = vec![take("library/bell.png"), take("library/bell-alt.png")];
        let mut found = vec![round];
        let edit = protocol::Iteration { file: "prompts/resources/bell-edit.png".into(), published: Some(1), fix: false };
        let assets = [
            crate::assets::Asset { first: "library/bell.png".into(), iterations: vec![edit.clone()], artifact: Some("Last order: keyframe, the bell".into()), in_use: true, outlined: true, ..Default::default() },
            crate::assets::Asset { first: "library/bell-alt.png".into(), artifact: Some("Last order: keyframe, the bell (2)".into()), ..Default::default() },
        ];
        fill_takes(&mut found, &assets);
        let (bell, alternative) = (&found[0].takes[0], &found[0].takes[1]);
        assert_eq!(bell.iterations, [edit]);
        assert!(bell.canon && bell.outlined && bell.artifact.as_deref() == Some("Last order: keyframe, the bell"));
        assert!(!alternative.canon && !alternative.outlined && alternative.artifact.is_none(), "a take not in use was shown in use: {alternative:?}");
    }

    /// Catches Generate again naming a third alternative "<step> (2) (2)",
    /// cutting a name whose last words are not a number, or naming one as
    /// Skills reads a keyframe's moment, "keyframe, wiping, 2".
    #[test]
    fn an_alternative_is_named_from_its_first() {
        assert_eq!(alternative_of("keyframe, wiping (2)"), ("keyframe, wiping", Some(2)));
        assert_eq!(alternative_of("final shot"), ("final shot", None));
        assert_eq!(alternative_of("final frame, waiting"), ("final frame, waiting", None));
        assert_eq!(alternative_of("final shot, 2"), ("final shot, 2", None));
        assert_eq!(alternative_of("Last order: final shot (3)"), ("Last order: final shot", Some(3)));
        assert_eq!(alternative_name("keyframe, wiping", 2), "keyframe, wiping (2)");
    }

    /// Catches a cell with two assets in use, two steps at one place, as
    /// Generate again leaves it, opening with only one step's rounds, or the
    /// chain's trial lost once two steps there follow its skill; and a
    /// step at another place taken in.
    #[test]
    fn a_cell_is_every_step_at_its_place() {
        let folder = std::env::temp_dir().join(format!("prompt-rounds-by-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("requests")).unwrap();
        let report = |id: &str, started: &str, step: Option<&str>, skill: &str| {
            let record = json!({ "id": id, "started": started, "prompt": "p", "sent": { "model": "m" }, "outputs": [format!("library/{id}.mp4")], "references": [], "chain": "the final shot", "step": step, "skill": skill, "setup": null });
            std::fs::write(folder.join(format!("requests/{id}.html")), format!("<script type=\"application/json\" id=\"request\">{record}</script>")).unwrap();
        };
        report("first", "2026-11-14T12:00:00Z", Some("final shot"), "create-final-animation");
        report("second", "2026-11-14T13:00:00Z", Some("final shot (2)"), "create-final-animation");
        report("trial", "2026-11-14T14:00:00Z", None, "create-final-animation");
        report("frame", "2026-11-14T11:00:00Z", Some("final frame"), "generate-keyframe");
        let mut canon = Canon::default();
        for (name, order, skill, artifact) in [("final shot", 5, "create-final-animation", "Last order: final shot"), ("final shot (2)", 5, "create-final-animation", "Last order: final shot (2)"), ("final frame", 3, "generate-keyframe", "Last order: final frame")] {
            let key = (String::from("the final shot"), name.to_string());
            canon.step_orders.insert(key.clone(), order);
            canon.step_skills.insert(key.clone(), skill.to_string());
            canon.step_artifacts.insert(key, artifact.to_string());
        }
        let step = StepRef { chain: String::from("the final shot"), step: String::from("final shot") };
        let found = rounds(&folder, "final", Some(&step), &[], &canon, &mut Reports::default());
        let ids: Vec<&str> = found.iter().map(|round| round.request.as_str()).collect();
        assert_eq!(ids, ["first", "second", "trial"], "the cell's two steps and its chain's trial, and not another place's step");
        assert_eq!(found[1].artifact.as_deref(), Some("Last order: final shot (2)"), "a round lost its own step's artifact");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a take of Last order opening with rounds it was not made in,
    /// such as every study made from the same texture: each take its
    /// requests made, as a cell's own, opens with its round and at most the
    /// rounds it was finished through. It only reads: set `LAST_ORDER` to
    /// its folder.
    #[test]
    fn each_of_last_orders_takes_opens_with_its_own_rounds() {
        let Ok(folder) = std::env::var("LAST_ORDER") else { return };
        let folder = PathBuf::from(folder);
        let mut reports = Reports::default();
        let takes: Vec<String> = std::fs::read_dir(folder.join("requests"))
            .unwrap()
            .flatten()
            .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
            .filter_map(|text| crate::skills_setup::record_of(&text))
            .flat_map(|record| record.get("outputs").and_then(Value::as_array).cloned().unwrap_or_default())
            .filter_map(|output| output.as_str().map(String::from))
            .collect();
        assert!(takes.len() > 400, "Last order's takes were not read: {}", takes.len());
        for take in &takes {
            let found = rounds(&folder, "a take", None, std::slice::from_ref(take), &Canon::default(), &mut reports);
            assert!(!found.is_empty() && found.len() <= 3, "{take} opens with {} rounds", found.len());
        }
        // The final shot's step: its nine takes and the LTX trial, then the
        // retime and the upscale finished from the approved take; and not
        // the start frame or the door's sound, made for its chain by skills
        // it does not follow.
        let step = StepRef { chain: String::from("the final shot"), step: String::from("final shot") };
        let mut canon = Canon::default();
        for (name, skill) in [("final shot", "create-final-animation"), ("final frame, waiting", "generate-keyframe"), ("final frame, wiping", "generate-keyframe"), ("plate, door closed", "generate-background"), ("plate, door open", "generate-background")] {
            canon.step_skills.insert((step.chain.clone(), name.to_string()), skill.to_string());
        }
        let shot = rounds(&folder, "a take", Some(&step), &[], &canon, &mut reports);
        let ids = found_ids(&shot);
        assert_eq!(ids.len(), 12, "{ids:?}");
        assert!(ids.contains(&"2026-11-14_20-41-final-shot-ltx"), "{ids:?}");
        assert_eq!(ids[10..], ["2026-11-15_08-12-final-shot-60fps", "2026-11-15_08-17-final-shot-60fps-4k"]);
    }

    fn found_ids(rounds: &[Round]) -> Vec<&str> {
        rounds.iter().map(|round| round.request.as_str()).collect()
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
        assert_eq!(refs[1]["artifact"], "the cook: take not made yet", "a reference with no file yet was dropped or moved");
        assert_eq!(refs[2]["file"], "library/new.png");
        // A field the person cleared is cleared, not kept.
        let read = json!({ "template": "t", "refs": r#"[{"file": "library/a.png", "role": "image 1", "field": "start_image"}]"# });
        let cleared = vec![
            CellPart::Text { text: String::from("t") },
            CellPart::Path { path: String::from("library/a.png"), media_type: String::from("image/png"), role: Some(String::from("image 1")), field: None },
        ];
        let (_, refs, _) = step_edits(&read, &Canon::default(), &cleared, None);
        let refs: Vec<Value> = serde_json::from_str(&refs.unwrap()).unwrap();
        assert!(refs[0].get("field").is_none(), "a cleared field kept its value: {refs:?}");
    }

    #[test]
    fn a_records_time_reads_as_seconds() {
        assert_eq!(epoch("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch("2026-11-16T12:00:00Z"), Some(1_794_830_400));
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
