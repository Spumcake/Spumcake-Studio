//! The prices in the setup file Skills' `request.py` reads, so a plan card
//! shows a cost, not "Cost unknown". Each is read from the service's own
//! price list for the model a plan names, kept in the data folder with the
//! day it was read, and read again at most once a day. A price the service
//! does not give as a flat amount per request or per minute is left out,
//! so it shows as unknown, never guessed. Each price in the setup file says
//! where it came from and when.
//!
//! - OpenRouter, for an image model: its endpoints' price for the image it
//!   makes, when that is per image or per request.
//! - OpenRouter, for a language model: its price per request, when it has
//!   one. A price per token alone is left out, since a request's tokens are
//!   not known before it runs.
//! - fal, with its key: its pricing list's base price, per image or video,
//!   or per second.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The prices read, in the data folder.
const CACHE: &str = "prices.json";

/// Where `request.py` sends OpenRouter requests, or a stand-in's address
/// for tests, from `INPAINTER_OPENROUTER_URL`.
fn openrouter_root() -> String {
    std::env::var("INPAINTER_OPENROUTER_URL").unwrap_or_else(|_| String::from("https://openrouter.ai/api/v1"))
}

/// fal's platform API, or a stand-in's for tests, from `INPAINTER_FAL_API_URL`.
fn fal_root() -> String {
    std::env::var("INPAINTER_FAL_API_URL").unwrap_or_else(|_| String::from("https://api.fal.ai/v1"))
}

fn today() -> String {
    crate::skills_setup::local_stamp().chars().take(10).collect()
}

fn read(data: &Path) -> Value {
    std::fs::read_to_string(data.join(CACHE)).ok().and_then(|text| serde_json::from_str(&text).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

/// The data folder, from the setup file the scripts are given.
pub(crate) fn data_of(vars: &[(String, String)]) -> Option<PathBuf> {
    let setup = vars.iter().find(|(name, _)| name == "SKILLS_SETUP")?;
    Path::new(&setup.1).parent().map(Path::to_path_buf)
}

/// The prices known, as the setup file gives them: by service, then by
/// model, each with where it came from and the day it was read.
pub(crate) fn known(data: &Path) -> Value {
    let mut prices = json!({});
    // The token prices the user set in Settings, which no list gives.
    for (service, model, per) in user_token_prices(data) {
        prices[service.as_str()][model.as_str()] = json!({ "per_million_tokens": per, "from": FROM_SETTINGS });
    }
    let today = today();
    for (service, models) in read(data).as_object().into_iter().flatten() {
        for (model, entry) in models.as_object().into_iter().flatten() {
            // Only a price read today: one older is read again before use.
            if entry.get("fetched").and_then(Value::as_str) != Some(today.as_str()) {
                continue;
            }
            if let Some(price) = entry.get("price").filter(|price| price.is_object()) {
                let mut price = price.clone();
                // The day the price was really read, which a failed read
                // today does not change.
                price["fetched"] = entry.get("read_on").or_else(|| entry.get("fetched")).cloned().unwrap_or(Value::Null);
                // A list's price never drops the user's token prices.
                if let Some(per) = prices.pointer(&format!("/{}/{}/per_million_tokens", pointer_part(service), pointer_part(model))).cloned() {
                    price["per_million_tokens"] = per;
                }
                prices[service.as_str()][model.as_str()] = price;
            }
        }
    }
    prices
}

/// Where the user's own token prices are said to come from.
const FROM_SETTINGS: &str = "your prices in Settings";

/// A key as one part of a JSON pointer.
fn pointer_part(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// The token prices the user set in Settings, read from the data folder's
/// settings: each endpoint's service, as Skills' setup file names it, its
/// model, and `per_million_tokens` with the prices given.
fn user_token_prices(data: &Path) -> Vec<(String, String, Value)> {
    let Some(settings) = std::fs::read_to_string(data.join("settings.json")).ok().and_then(|text| serde_json::from_str::<crate::workspace::Settings>(&text).ok()) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for price in &settings.token_prices {
        let Some(endpoint) = settings.endpoints.iter().find(|endpoint| endpoint.id == price.endpoint) else { continue };
        let Some(service) = serde_json::to_value(endpoint.kind).ok().and_then(|kind| kind.as_str().map(String::from)) else { continue };
        let mut per = json!({});
        for (key, value) in [("input_text", price.input_text), ("input_image", price.input_image), ("output_image", price.output_image)] {
            if let Some(value) = value {
                per[key] = json!(value);
            }
        }
        if per.as_object().is_some_and(|per| !per.is_empty()) {
            found.push((service, price.model.clone(), per));
        }
    }
    found
}

/// Where a plan's listed cost came from, for the card to show beside it:
/// today's price list, with what that price leaves out, or, when the cost
/// is not today's listed price, the spec's or step's own price.
pub(crate) fn note_for(data: &Path, plan: &Value) -> Option<String> {
    let estimate = plan.get("estimate").and_then(Value::as_str)?;
    // An estimate from the project's bills or the user's prices says so itself.
    if !estimate.starts_with("about $") || estimate.contains("earlier request") || estimate.contains(FROM_YOUR_PRICES) {
        return None;
    }
    let spec_own = || Some(String::from("comes from the request's own endpoint, not today's price list"));
    let kind = plan.pointer("/endpoint/kind").and_then(Value::as_str).unwrap_or("");
    let service = match kind.split('-').next() {
        Some("openrouter") => "openrouter",
        Some("fal") => "fal",
        _ => return spec_own(),
    };
    let model = plan.pointer("/endpoint/model").and_then(Value::as_str).unwrap_or("");
    let prices = known(data);
    let Some(price) = prices.get(service).and_then(|models| models.get(model)) else { return spec_own() };
    // request.py prints a price per request with `money`, and a price per
    // minute as Python prints the number, which is Rust's shortest form too.
    let listed = price.get("per_request").and_then(Value::as_f64).is_some_and(|each| estimate == format!("about {} a request", money(each)))
        || price.get("per_minute").and_then(Value::as_f64).is_some_and(|minute| estimate.contains(&format!("at ${minute} a minute")));
    if !listed {
        return spec_own();
    }
    let read = price.get("fetched").and_then(Value::as_str).unwrap_or("");
    let leaves_out = price.get("note").and_then(Value::as_str).map(|note| format!("; it {note}")).unwrap_or_default();
    Some(format!("is the price list's, read {read}{leaves_out}"))
}

/// Makes sure the price of the model a plan or spec names was read today.
/// Returns whether a price was read now, so the plan is made again with it.
pub(crate) fn ensure(data: &Path, plan: &Value, vars: &[(String, String)]) -> bool {
    ensure_at(data, plan, vars, &openrouter_root(), &fal_root())
}

fn ensure_at(data: &Path, plan: &Value, vars: &[(String, String)], openrouter: &str, fal: &str) -> bool {
    let plan = match plan {
        Value::Array(plans) => plans.first().unwrap_or(&Value::Null),
        plan => plan,
    };
    let kind = plan.pointer("/endpoint/kind").and_then(Value::as_str).unwrap_or("");
    let model = plan.pointer("/endpoint/model").and_then(Value::as_str).unwrap_or("").trim();
    let service = match kind.split('-').next() {
        Some("openrouter") => "openrouter",
        Some("fal") => "fal",
        _ => return false,
    };
    if model.is_empty() || model == "stand-in" {
        return false;
    }
    let mut cache = read(data);
    let today = today();
    if cache.get(service).and_then(|models| models.get(model)).and_then(|entry| entry.get("fetched")).and_then(Value::as_str) == Some(today.as_str()) {
        return false;
    }
    let var = |name: &str| vars.iter().find(|(known, _)| known == name).map(|(_, value)| value.clone());
    let fetched = match (service, kind) {
        ("openrouter", "openrouter-image") => openrouter_image(openrouter, model),
        ("openrouter", _) => openrouter_text(openrouter, model),
        // fal's list needs its key: with none, nothing is asked or kept, so
        // a key added later is used the same day.
        _ => match var("FAL_KEY") {
            Some(key) => fal_price(fal, model, &key),
            None => return false,
        },
    };
    let last = cache.get(service).and_then(|models| models.get(model)).cloned().unwrap_or(Value::Null);
    let (price, read_on) = match fetched {
        Ok(price) => (price, today.clone()),
        // The service could not be reached: the last price read is kept,
        // with the day it was really read, rather than lost.
        Err(_) => {
            let read_on = last.get("read_on").or_else(|| last.get("fetched")).and_then(Value::as_str).unwrap_or("an earlier day").to_string();
            (last.get("price").filter(|price| price.is_object()).cloned(), read_on)
        }
    };
    // A price not found is kept too, so it is not asked for again today.
    cache[service][model] = json!({ "fetched": today, "read_on": read_on, "price": price });
    let _ = std::fs::write(data.join(CACHE), serde_json::to_string_pretty(&cache).unwrap_or_default());
    price.is_some()
}

/// Every model the skills' `models.json` name for a service, such as
/// "openai", once each, with its name: the models whose token prices an
/// endpoint of that service takes.
pub(crate) fn service_models(skills: &[crate::skills_setup::Skill], service: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for skill in skills {
        let listed: Value = std::fs::read_to_string(skill.folder.join("models.json")).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null);
        for entry in listed.as_array().into_iter().flatten() {
            let text = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
            if text("service") == service && !text("model").is_empty() && !found.iter().any(|(model, _)| *model == text("model")) {
                let name = if text("name").is_empty() { text("model") } else { text("name") };
                found.push((text("model"), name));
            }
        }
    }
    found
}

/// The service a skill's `models.json` names for a model, such as "fal".
pub(crate) fn local_service(skills: &[crate::skills_setup::Skill], model: &str) -> Option<String> {
    skills.iter().find_map(|skill| {
        let listed: Value = std::fs::read_to_string(skill.folder.join("models.json")).ok().and_then(|text| serde_json::from_str(&text).ok())?;
        listed.as_array()?.iter().find(|entry| entry.get("model").and_then(Value::as_str) == Some(model))?.get("service")?.as_str().map(String::from)
    })
}

/// A model's name from a `models.json` beside a skill's `SKILL.md`, a list
/// of `{service, model, name}`, as the Studio-as-a-setup contract's
/// Formats table settled it on 2026-11-17. Read from this computer, never
/// the network; None when no skill names it.
pub(crate) fn local_name(skills: &[crate::skills_setup::Skill], model: &str) -> Option<String> {
    let named = |entry: &Value| (entry.get("model").and_then(Value::as_str) == Some(model)).then(|| entry.get("name").and_then(Value::as_str)).flatten().map(|name| name.trim().to_string()).filter(|name| !name.is_empty());
    skills.iter().find_map(|skill| {
        let listed: Value = std::fs::read_to_string(skill.folder.join("models.json")).ok().and_then(|text| serde_json::from_str(&text).ok())?;
        listed.as_array()?.iter().find_map(named)
    })
}

/// Every `pricing` list in a reply, wherever it is.
fn pricing_lists(value: &Value, found: &mut Vec<Value>) {
    match value {
        Value::Object(fields) => {
            for (name, field) in fields {
                if name == "pricing" && field.is_array() {
                    found.extend(field.as_array().into_iter().flatten().cloned());
                } else {
                    pricing_lists(field, found);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| pricing_lists(item, found)),
        _ => {}
    }
}

/// An image model's price for the image it makes, from its endpoints: the
/// highest, so the card never shows less than may be billed.
fn openrouter_image(root: &str, model: &str) -> Result<Option<Value>, String> {
    let url = format!("{root}/images/models/{model}/endpoints");
    let reply = assistant::web::get_json(&url, &[])?;
    Ok(image_price(&reply, url))
}

fn image_price(reply: &Value, url: String) -> Option<Value> {
    let mut entries = Vec::new();
    pricing_lists(reply, &mut entries);
    let cost = |entry: &Value| entry.get("cost_usd").and_then(Value::as_f64).unwrap_or(0.0);
    let text = |entry: &Value, key: &str| entry.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let flat = entries.iter().filter(|entry| text(entry, "billable") == "output_image" && matches!(text(entry, "unit").as_str(), "image" | "request")).map(cost).fold(None, |most: Option<f64>, cost| Some(most.map_or(cost, |most| most.max(cost))))?;
    if flat <= 0.0 {
        return None;
    }
    let mut price = json!({ "per_request": flat, "from": url });
    let others: Vec<String> = entries.iter().filter(|entry| text(entry, "billable") != "output_image" && cost(entry) > 0.0).map(|entry| format!("{} per {}", text(entry, "billable"), text(entry, "unit"))).collect();
    if !others.is_empty() {
        price["note"] = json!(format!("covers the image made only; also billed: {}", others.join(", ")));
    }
    Some(price)
}

/// A language model's price per request, when OpenRouter gives one.
fn openrouter_text(root: &str, model: &str) -> Result<Option<Value>, String> {
    let url = format!("{root}/models");
    let reply = assistant::web::get_json(&url, &[])?;
    Ok(text_price(&reply, url, model))
}

fn text_price(reply: &Value, url: String, model: &str) -> Option<Value> {
    let entry = reply.get("data")?.as_array()?.iter().find(|entry| entry.get("id").and_then(Value::as_str) == Some(model))?;
    let per_request: f64 = entry.pointer("/pricing/request").and_then(Value::as_str)?.parse().ok()?;
    (per_request > 0.0).then(|| json!({ "per_request": per_request, "from": url, "note": "covers the request only; its tokens are billed besides" }))
}

/// fal's base price for an endpoint, read with the key.
fn fal_price(root: &str, model: &str, key: &str) -> Result<Option<Value>, String> {
    let url = format!("{root}/models/pricing?endpoint_id={model}");
    let authorization = format!("Key {key}");
    let reply = assistant::web::get_json(&url, &[("Authorization", authorization.as_str())])?;
    Ok(fal_entry_price(&reply, url, model))
}

fn fal_entry_price(reply: &Value, url: String, model: &str) -> Option<Value> {
    let entry = reply.get("prices")?.as_array()?.iter().find(|entry| entry.get("endpoint_id").and_then(Value::as_str) == Some(model))?;
    if entry.get("currency").and_then(Value::as_str).unwrap_or("USD") != "USD" {
        return None;
    }
    let price = entry.get("unit_price").and_then(Value::as_f64).filter(|price| *price > 0.0)?;
    let note = "is fal's base price, without its adjustments for size or length";
    match entry.get("unit").and_then(Value::as_str)? {
        "image" | "video" | "request" => Some(json!({ "per_request": price, "from": url, "note": note })),
        "second" | "seconds" => Some(json!({ "per_minute": price * 60.0, "from": url, "note": note })),
        "minute" | "minutes" => Some(json!({ "per_minute": price, "from": url, "note": note })),
        _ => None,
    }
}

/// The settings that change what a request costs, compared between a plan
/// and the requests before it.
const SIZE_SETTINGS: [&str; 10] = ["size", "aspect_ratio", "quality", "n", "num_images", "resolution", "image_size", "duration", "width", "height"];

/// What one earlier request's report says it was billed, kept by the
/// report's file and time, so a report is read once.
#[derive(Clone)]
struct Billed {
    service: String,
    model: String,
    sizes: Vec<Value>,
    /// How many references were sent, which change what a request costs
    /// as much as its size does.
    references: usize,
    cost: f64,
}

type Reports = std::collections::HashMap<PathBuf, (Option<std::time::SystemTime>, Option<Billed>)>;

fn reports() -> &'static std::sync::Mutex<Reports> {
    static REPORTS: std::sync::OnceLock<std::sync::Mutex<Reports>> = std::sync::OnceLock::new();
    REPORTS.get_or_init(Default::default)
}

fn sizes(settings: Option<&Value>) -> Vec<Value> {
    SIZE_SETTINGS.iter().map(|key| settings.and_then(|settings| settings.get(*key)).cloned().unwrap_or(Value::Null)).collect()
}

fn service_name(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase()
}

/// What a report says it cost: a cost the service itself reported, or one
/// `request.py` computed from the tokens the service reported using, never
/// an estimate made before it ran, and never a stand-in's.
fn billed_of(record: &Value) -> Option<Billed> {
    if record.get("stand_in").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let computed = || {
        let said = record.pointer("/result/cost").and_then(Value::as_str)?;
        let amount = said.strip_prefix("about $")?.split(|c: char| !(c.is_ascii_digit() || c == '.')).next()?;
        said.contains("computed from the token usage").then(|| amount.parse::<f64>().ok()).flatten()
    };
    let cost = record.pointer("/result/usage/cost").and_then(Value::as_f64).or_else(computed).filter(|cost| *cost > 0.0)?;
    Some(Billed {
        service: service_name(record.pointer("/sent/service").and_then(Value::as_str)?),
        model: record.pointer("/sent/model").and_then(Value::as_str)?.to_string(),
        sizes: sizes(record.pointer("/sent/settings")),
        references: record.get("references").and_then(Value::as_array).map_or(0, Vec::len),
        cost,
    })
}

/// A price as `request.py` prints one: with enough places that it never
/// reads as zero, and within 5% of the amount.
fn money(amount: f64) -> String {
    let mut places = 2;
    let rounded = |places: i32| (amount * 10f64.powi(places)).round() / 10f64.powi(places);
    while places < 8 && (rounded(places) == 0.0 || (rounded(places) - amount).abs() / amount > 0.05) {
        places += 1;
    }
    format!("${:.*}", places as usize, amount)
}

/// For a plan whose price is unknown, an estimate from the project's own
/// record: the median of what earlier requests to the same service and
/// model, at the same size settings and with as many references, were
/// billed. None when there are none.
pub(crate) fn from_history(folder: &Path, plan: &Value) -> Option<String> {
    let kind = plan.pointer("/endpoint/kind").and_then(Value::as_str)?;
    let service = service_name(kind.split('-').next()?);
    let model = plan.pointer("/endpoint/model").and_then(Value::as_str)?;
    let wanted = sizes(plan.pointer("/endpoint/settings"));
    let references = plan.get("references").and_then(Value::as_array).map_or(0, Vec::len);
    let mut costs: Vec<f64> = Vec::new();
    let mut known = reports().lock().ok()?;
    for entry in std::fs::read_dir(folder.join("requests")).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("html") {
            continue;
        }
        let time = entry.metadata().and_then(|meta| meta.modified()).ok();
        let billed = match known.get(&path).filter(|(at, _)| *at == time) {
            Some((_, billed)) => billed.clone(),
            None => {
                let billed = std::fs::read_to_string(&path).ok().as_deref().and_then(crate::skills_setup::record_of).as_ref().and_then(billed_of);
                known.insert(path, (time, billed.clone()));
                billed
            }
        };
        if let Some(billed) = billed
            && billed.service == service
            && billed.model == model
            && billed.sizes == wanted
            && billed.references == references
        {
            costs.push(billed.cost);
        }
    }
    if costs.is_empty() {
        return None;
    }
    costs.sort_by(f64::total_cmp);
    let middle = costs.len() / 2;
    let median = if costs.len() % 2 == 0 { (costs[middle - 1] + costs[middle]) / 2.0 } else { costs[middle] };
    let noun = if costs.len() == 1 { "earlier request" } else { "earlier requests" };
    Some(format!("about {} · from {} {noun}", money(median), costs.len()))
}

/// A plan's estimate, from the project's own record when its price is
/// unknown: the plan card and the cost line both show it.
pub(crate) fn fill_from_history(folder: &Path, plan: &mut Value) {
    let unknown = plan.get("estimate").and_then(Value::as_str).is_none_or(|estimate| estimate.starts_with("unknown"));
    if unknown
        && plan.get("stand_in").and_then(Value::as_bool) != Some(true)
        && let Some(estimate) = from_history(folder, plan)
    {
        plan["estimate"] = json!(estimate);
    }
}

/// What one earlier request to a model priced by its tokens used, from its
/// report's own token counts: its prompt's text, the images it sent, and
/// the images it made, with the settings that change what it makes.
#[derive(Clone)]
struct Used {
    model: String,
    made: (Value, Value, u64),
    references: usize,
    prompt: usize,
    text_in: f64,
    image_in: f64,
    out: f64,
}

type Usages = std::collections::HashMap<PathBuf, (Option<std::time::SystemTime>, Option<Used>)>;

fn usages() -> &'static std::sync::Mutex<Usages> {
    static USAGES: std::sync::OnceLock<std::sync::Mutex<Usages>> = std::sync::OnceLock::new();
    USAGES.get_or_init(Default::default)
}

/// The settings that change how many tokens an image made uses: its size,
/// its quality, and how many are made, one when none is said.
fn made_by(settings: Option<&Value>) -> (Value, Value, u64) {
    let get = |key: &str| settings.and_then(|settings| settings.get(key)).cloned().unwrap_or(Value::Null);
    let count = ["n", "num_images"].iter().find_map(|key| settings.and_then(|settings| settings.get(*key)).and_then(Value::as_u64)).unwrap_or(1);
    (get("size"), get("quality"), count)
}

fn used_of(record: &Value) -> Option<Used> {
    if record.get("stand_in").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let usage = record.pointer("/result/usage")?;
    let details = usage.get("input_tokens_details");
    let text_in = details.and_then(|details| details.get("text_tokens")).and_then(Value::as_f64)?;
    let image_in = details.and_then(|details| details.get("image_tokens")).and_then(Value::as_f64).unwrap_or(0.0);
    let out = usage.get("output_tokens").and_then(Value::as_f64)?;
    // A report keeps what it sent either under its settings or beside them.
    let sent = record.get("sent")?;
    let settings = sent.get("settings").filter(|settings| settings.is_object()).unwrap_or(sent);
    Some(Used {
        model: sent.get("model").and_then(Value::as_str)?.to_string(),
        made: made_by(Some(settings)),
        references: record.get("references").and_then(Value::as_array).map_or(0, Vec::len),
        prompt: record.get("prompt").and_then(Value::as_str).map_or(0, |prompt| prompt.chars().count()),
        text_in,
        image_in,
        out,
    })
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 0 { (values[middle - 1] + values[middle]) / 2.0 } else { values[middle] })
}

/// The mark on an estimate made from the user's token prices.
pub(crate) const FROM_YOUR_PRICES: &str = "estimate from your prices";
/// The card's second line when a model priced by its tokens has no prices.
pub(crate) const SET_PRICES: &str = "Set its prices in Settings, under its endpoint.";

/// A model's token counts as its skill's `models.json` gives them, the
/// service's own: `tokens: {per_image_in, per_image_out: {"<size>
/// <quality>": n}}`, read from the skill folders the data folder's
/// settings list. None when no skill gives them.
fn listed_tokens(data: &Path, service: &str, model: &str) -> Option<Value> {
    let settings: crate::workspace::Settings = std::fs::read_to_string(data.join("settings.json")).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
    let skills = crate::skills_setup::listed_skills(&crate::skills_checkout::skill_folders(&settings.skill_folders));
    skills.iter().find_map(|skill| {
        let listed: Value = std::fs::read_to_string(skill.folder.join("models.json")).ok().and_then(|text| serde_json::from_str(&text).ok())?;
        listed
            .as_array()?
            .iter()
            .find(|entry| entry.get("service").and_then(Value::as_str) == Some(service) && entry.get("model").and_then(Value::as_str) == Some(model))?
            .get("tokens")
            .filter(|tokens| tokens.is_object())
            .cloned()
    })
}

/// Whether a plan to a model priced by its tokens waits on the user's
/// prices: its cost is unknown, and none are set for it.
pub(crate) fn needs_prices(data: &Path, plan: &Value) -> bool {
    let unknown = plan.get("estimate").and_then(Value::as_str).is_none_or(|estimate| estimate.starts_with("unknown"));
    let service = plan.pointer("/endpoint/kind").and_then(Value::as_str).and_then(|kind| kind.split('-').next()).unwrap_or("");
    let model = plan.pointer("/endpoint/model").and_then(Value::as_str).unwrap_or("");
    unknown && service == "openai" && plan.get("stand_in").and_then(Value::as_bool) != Some(true) && !user_token_prices(data).iter().any(|(known, priced, _)| known == service && priced == model)
}

/// For a plan to a model priced by its tokens, an estimate from the user's
/// token prices and what it will send: its prompt's text, as the project's
/// earlier requests to the model counted it, or a token for every four
/// characters; each image it sends, and each image it makes at its size and
/// quality, as those requests counted them, or else as the model's skill's
/// `models.json` gives them. None when a price or a count it needs is
/// unknown.
pub(crate) fn from_token_prices(data: &Path, folder: &Path, plan: &Value) -> Option<String> {
    let service = plan.pointer("/endpoint/kind").and_then(Value::as_str)?.split('-').next()?.to_string();
    let model = plan.pointer("/endpoint/model").and_then(Value::as_str)?;
    let (_, _, per) = user_token_prices(data).into_iter().find(|(known, priced, _)| *known == service && priced == model)?;
    let price = |key: &str| per.get(key).and_then(Value::as_f64);
    let prompt = plan.get("prompt").and_then(Value::as_str).map_or(0, |prompt| prompt.chars().count());
    let references = plan.get("references").and_then(Value::as_array).map_or(0, Vec::len);
    let made = made_by(plan.pointer("/endpoint/settings"));
    let mut known = usages().lock().ok()?;
    let mut earlier: Vec<Used> = Vec::new();
    for entry in std::fs::read_dir(folder.join("requests")).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("html") {
            continue;
        }
        let time = entry.metadata().and_then(|meta| meta.modified()).ok();
        let used = match known.get(&path).filter(|(at, _)| *at == time) {
            Some((_, used)) => used.clone(),
            None => {
                let used = std::fs::read_to_string(&path).ok().as_deref().and_then(crate::skills_setup::record_of).as_ref().and_then(used_of);
                known.insert(path, (time, used.clone()));
                used
            }
        };
        if let Some(used) = used.filter(|used| used.model == model) {
            earlier.push(used);
        }
    }
    let per_character = median(earlier.iter().filter(|used| used.prompt > 0).map(|used| used.text_in / used.prompt as f64).collect()).unwrap_or(0.25);
    let text = prompt as f64 * per_character;
    drop(known);
    let listed = listed_tokens(data, &service, model);
    let per_image_in = median(earlier.iter().filter(|used| used.references > 0).map(|used| used.image_in / used.references as f64).collect()).or_else(|| listed.as_ref()?.get("per_image_in")?.as_f64());
    let image = if references == 0 { 0.0 } else { per_image_in? * references as f64 };
    let listed_out = || {
        let key = [made.0.as_str().unwrap_or(""), made.1.as_str().unwrap_or("")].join(" ");
        Some(listed.as_ref()?.get("per_image_out")?.get(key.trim())?.as_f64()? * made.2 as f64)
    };
    let out = median(earlier.iter().filter(|used| used.made == made).map(|used| used.out).collect()).or_else(listed_out)?;
    let mut cost = 0.0;
    for (tokens, key) in [(text, "input_text"), (image, "input_image"), (out, "output_image")] {
        if tokens > 0.0 {
            cost += tokens * price(key)? / 1e6;
        }
    }
    (cost > 0.0).then(|| format!("about {} · {FROM_YOUR_PRICES}", money(cost)))
}

/// A plan's estimate, from the user's token prices when no list gives its
/// price.
pub(crate) fn fill_from_token_prices(data: &Path, folder: &Path, plan: &mut Value) {
    let unknown = plan.get("estimate").and_then(Value::as_str).is_none_or(|estimate| estimate.starts_with("unknown"));
    if unknown
        && plan.get("stand_in").and_then(Value::as_bool) != Some(true)
        && let Some(estimate) = from_token_prices(data, folder, plan)
    {
        plan["estimate"] = json!(estimate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Catches a model priced by its tokens left at "Cost unknown" once the
    /// user has set its prices, an estimate that ignores the prompt or the
    /// images it sends, one made from another size's images or a
    /// stand-in's counts, the user's prices missing from the setup file, and
    /// a note on the card saying the price came from somewhere else.
    #[test]
    fn a_token_priced_model_is_estimated_from_the_users_prices() {
        let root = std::env::temp_dir().join(format!("prices-tokens-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (data, folder) = (root.join("data"), root.join("project"));
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(folder.join("requests")).unwrap();
        let model = "gpt-image-2.6-sunburst";
        let report = |id: &str, size: &str, out: u64, stand_in: bool| {
            let record = json!({
                "id": id, "prompt": "p".repeat(400), "stand_in": stand_in,
                "references": [{ "file": "library/a.png" }, { "file": "library/b.png" }],
                "sent": { "model": model, "size": size, "quality": "high", "n": 1, "service": "OpenAI" },
                "result": { "cost": "not recorded", "usage": { "input_tokens": 3100, "input_tokens_details": { "text_tokens": 100, "image_tokens": 3000 }, "output_tokens": out } },
            });
            std::fs::write(folder.join(format!("requests/{id}.html")), format!("<script type=\"application/json\" id=\"request\">{record}</script>")).unwrap();
        };
        report("a", "1536x864", 1000, false);
        report("b", "1536x864", 1200, false);
        report("other size", "1024x1024", 9000, false);
        report("stand-in", "1536x864", 50_000, true);
        let plan = json!({
            "endpoint": { "kind": "openai-image-edit", "model": model, "settings": { "size": "1536x864", "quality": "high", "seed": 7 } },
            "prompt": "p".repeat(800),
            "references": [{ "order": 1 }, { "order": 2 }, { "order": 3 }],
            "estimate": "unknown: no price for this endpoint",
        });
        assert_eq!(from_token_prices(&data, &folder, &plan), None, "an estimate with no prices set");
        let settings = json!({
            "endpoints": [{ "id": "images", "name": "Images", "kind": "openai", "address": "http://images.test/v1" }],
            "token_prices": [{ "endpoint": "images", "model": model, "input_text": 5.0, "input_image": 10.0, "output_image": 40.0 }],
        });
        std::fs::write(data.join("settings.json"), settings.to_string()).unwrap();
        // 800 characters at a quarter token each, three images at 1500
        // tokens each, and the median of 1000 and 1200 tokens made:
        // 200 × $5 + 4500 × $10 + 1100 × $40, a million tokens each.
        let mut filled = plan.clone();
        fill_from_token_prices(&data, &folder, &mut filled);
        assert_eq!(filled["estimate"], json!(format!("about $0.09 · {FROM_YOUR_PRICES}")));
        assert_eq!(note_for(&data, &filled), None, "the card said an estimate from the user's prices came from elsewhere");
        let mut larger = plan.clone();
        larger["endpoint"]["settings"]["size"] = json!("2048x2048");
        assert_eq!(from_token_prices(&data, &folder, &larger), None, "an image of a size never made was estimated");
        assert_eq!(known(&data)["openai"][model]["per_million_tokens"]["output_image"], json!(40.0), "the setup file lost the user's prices");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Catches a new project's first request to a model priced by its
    /// tokens left at "Cost unknown" with its prices set, when its skill's
    /// `models.json` gives the model's token counts; and the card's line
    /// pointing to Settings shown once prices are set, or missing before.
    #[test]
    fn a_new_projects_first_request_is_estimated_from_its_models_counts() {
        let root = std::env::temp_dir().join(format!("prices-new-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (data, folder, skill) = (root.join("data"), root.join("project"), root.join("skills").join("generate-keyframe"));
        for made in [&data, &folder, &skill] {
            std::fs::create_dir_all(made).unwrap();
        }
        let model = "gpt-image-2.6-sunburst";
        std::fs::write(skill.join("SKILL.md"), "---\nname: generate-keyframe\ndescription: Make keyframes.\n---\n").unwrap();
        std::fs::write(skill.join("models.json"), json!([{ "service": "openai", "model": model, "name": "OpenAI GPT-Image-2.6 Sunburst", "tokens": { "per_image_in": 1296, "per_image_out": { "1536x864 high": 1078, "1344x768 high": 983 } } }]).to_string()).unwrap();
        let plan = json!({
            "endpoint": { "kind": "openai-image-edit", "model": model, "settings": { "size": "1536x864", "quality": "high" } },
            "prompt": "p".repeat(400),
            "references": [{ "order": 1 }, { "order": 2 }],
            "estimate": "unknown: no price for this endpoint",
        });
        let mut settings = json!({
            "endpoints": [{ "id": "images", "name": "Images", "kind": "openai", "address": "http://images.test/v1" }],
            "skill_folders": [root.join("skills").display().to_string()],
        });
        std::fs::write(data.join("settings.json"), settings.to_string()).unwrap();
        assert!(needs_prices(&data, &plan), "a card with no prices did not point to Settings");
        settings["token_prices"] = json!([{ "endpoint": "images", "model": model, "input_text": 5.0, "input_image": 8.0, "output_image": 30.0 }]);
        std::fs::write(data.join("settings.json"), settings.to_string()).unwrap();
        assert!(!needs_prices(&data, &plan), "a card with prices set still pointed to Settings");
        // 400 characters at a quarter token each, two images at 1296
        // tokens each, and one image made at 1078 tokens: 100 × $5 +
        // 2592 × $8 + 1078 × $30, a million tokens each.
        let mut filled = plan.clone();
        fill_from_token_prices(&data, &folder, &mut filled);
        assert_eq!(filled["estimate"], json!(format!("about $0.054 · {FROM_YOUR_PRICES}")), "a project with no requests yet was not estimated");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Catches an estimate made from requests to another model or size, a
    /// stand-in's run, or an estimate rather than a bill; and a mean taken
    /// for the median.
    #[test]
    fn an_unknown_price_comes_from_the_projects_own_bills() {
        let folder = std::env::temp_dir().join(format!("prices-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("requests")).unwrap();
        let report = |id: &str, model: &str, quality: &str, cost: Value, stand_in: bool| {
            let record = json!({ "id": id, "stand_in": stand_in, "references": [{ "file": "library/layout.png" }], "sent": { "service": "OpenRouter", "model": model, "settings": { "aspect_ratio": "16:9", "quality": quality, "seed": 3 } }, "result": { "cost": "as reported", "usage": { "cost": cost } } });
            std::fs::write(folder.join(format!("requests/{id}.html")), format!("<script type=\"application/json\" id=\"request\">{record}</script>")).unwrap();
        };
        report("a", "openai/gpt-image-2.5-sunburst", "high", json!(0.03), false);
        report("b", "openai/gpt-image-2.5-sunburst", "high", json!(0.04), false);
        report("c", "openai/gpt-image-2.5-sunburst", "high", json!(0.20), false);
        report("low", "openai/gpt-image-2.5-sunburst", "low", json!(0.01), false);
        report("other", "google/gemini-3-pro-image", "high", json!(0.5), false);
        report("stand-in", "openai/gpt-image-2.5-sunburst", "high", json!(9.0), true);
        report("estimate", "openai/gpt-image-2.5-sunburst", "high", json!("about $1"), false);
        // A cost request.py computed from the tokens used counts; an
        // estimate made before the request ran does not.
        let computed = json!({ "id": "computed", "sent": { "service": "OpenAI", "model": "gpt-image-2.6-sunburst", "settings": { "quality": "high" } }, "result": { "cost": "about $0.21, computed from the token usage and the setup's prices, not a bill", "usage": { "output_tokens": 4000 } } });
        std::fs::write(folder.join("requests/computed.html"), format!("<script type=\"application/json\" id=\"request\">{computed}</script>")).unwrap();
        let estimated = json!({ "id": "estimated", "sent": { "service": "OpenAI", "model": "gpt-image-2.6-sunburst", "settings": { "quality": "high" } }, "result": { "cost": "estimate $0.30; billed not returned" } });
        std::fs::write(folder.join("requests/estimated.html"), format!("<script type=\"application/json\" id=\"request\">{estimated}</script>")).unwrap();
        let mut openai = json!({ "endpoint": { "kind": "openai-image", "model": "gpt-image-2.6-sunburst", "settings": { "quality": "high" } }, "estimate": "unknown: 4000 tokens used" });
        fill_from_history(&folder, &mut openai);
        assert_eq!(openai["estimate"], "about $0.21 · from 1 earlier request");
        let mut plan = json!({ "endpoint": { "kind": "openrouter-image", "model": "openai/gpt-image-2.5-sunburst", "settings": { "aspect_ratio": "16:9", "quality": "high", "seed": 9 } }, "references": [{ "file": "library/a.png" }], "estimate": "unknown: no price for this endpoint" });
        fill_from_history(&folder, &mut plan);
        assert_eq!(plan["estimate"], "about $0.04 · from 3 earlier requests");
        // Twelve references cost more than one: one-reference bills are not
        // its estimate.
        let mut many = plan.clone();
        many["references"] = json!(vec![json!({ "file": "library/a.png" }); 12]);
        many["estimate"] = json!("unknown: no price for this endpoint");
        fill_from_history(&folder, &mut many);
        assert_eq!(many["estimate"], "unknown: no price for this endpoint", "a request with twelve references was estimated from one-reference bills");
        let mut none = json!({ "endpoint": { "kind": "openrouter-image", "model": "vendor/never-used" }, "estimate": "unknown: no price for this endpoint" });
        fill_from_history(&folder, &mut none);
        assert_eq!(none["estimate"], "unknown: no price for this endpoint");
        assert_eq!(money(0.0337), "$0.034");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A stand-in for the services' price lists: each path asked gets its
    /// reply, and every request asked is returned. Tests never call a
    /// service.
    fn stand_in(replies: Vec<(&'static str, String)>, count: usize) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let serve = std::thread::spawn(move || {
            let mut asked = Vec::new();
            for _ in 0..count {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
                    request.push(byte[0]);
                }
                let request = String::from_utf8_lossy(&request).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
                let body = replies.iter().find(|(prefix, _)| path.starts_with(prefix)).map(|(_, body)| body.clone()).unwrap_or_else(|| String::from("{}"));
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                asked.push(request.to_lowercase());
            }
            asked
        });
        (root, serve)
    }

    /// Catches a video model shown by the tail of its id when a skill
    /// names it.
    #[test]
    fn a_skill_names_its_models_first() {
        let folder = std::env::temp_dir().join(format!("prices-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("models.json"), json!([{ "service": "fal", "model": "minimax/h3-max/reference-to-video", "name": "MiniMax H3 Max" }]).to_string()).unwrap();
        let skill = crate::skills_setup::Skill { name: String::from("generate-shot"), description: String::new(), implements: Vec::new(), helps: Vec::new(), folder: folder.clone(), root: folder.clone() };
        assert_eq!(local_name(std::slice::from_ref(&skill), "minimax/h3-max/reference-to-video").as_deref(), Some("MiniMax H3 Max"));
        assert_eq!(local_name(&[skill], "other/model"), None);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a price guessed from per-token prices, a fal price read
    /// without its key, a price asked for again the same day, and a price
    /// in the setup file without where it came from.
    #[test]
    fn prices_come_from_the_services_lists_and_say_where() {
        let data = std::env::temp_dir().join(format!("prices-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        let image = json!({ "data": { "endpoints": [{ "pricing": [
            { "billable": "input_text", "unit": "token", "cost_usd": 0.000005 },
            { "billable": "output_image", "unit": "image", "cost_usd": 0.04 },
        ] }, { "pricing": [{ "billable": "output_image", "unit": "image", "cost_usd": 0.05 }] }] } });
        let tokens = json!({ "data": { "endpoints": [{ "pricing": [{ "billable": "output_image", "unit": "token", "cost_usd": 0.00003 }] }] } });
        let fal = json!({ "prices": [{ "endpoint_id": "minimax/h3-max/reference-to-video", "unit_price": 0.08, "unit": "second", "currency": "USD" }], "has_more": false });
        let (root, serve) = stand_in(
            vec![
                ("/images/models/vendor/flat-image/endpoints", image.to_string()),
                ("/images/models/vendor/token-image/endpoints", tokens.to_string()),
                ("/models/pricing", fal.to_string()),
            ],
            3,
        );
        let plan = |kind: &str, model: &str| json!({ "endpoint": { "kind": kind, "model": model } });
        let vars = vec![(String::from("FAL_KEY"), String::from("stand-in-key"))];
        assert!(ensure_at(&data, &plan("openrouter-image", "vendor/flat-image"), &vars, &root, &root));
        assert!(!ensure_at(&data, &plan("openrouter-image", "vendor/token-image"), &vars, &root, &root), "a price per token was made into a price per request");
        assert!(ensure_at(&data, &plan("fal", "minimax/h3-max/reference-to-video"), &vars, &root, &root));
        // Asked again the same day, nothing is fetched: the stand-in
        // answers three requests only.
        assert!(!ensure_at(&data, &plan("openrouter-image", "vendor/flat-image"), &vars, &root, &root));
        assert!(!ensure_at(&data, &plan("fal", "other/model"), &[], &root, &root), "fal's price was read without its key");
        let asked = serve.join().unwrap();
        assert!(asked.iter().any(|request| request.contains("authorization: key stand-in-key")));
        let prices = known(&data);
        assert_eq!(prices["openrouter"]["vendor/flat-image"]["per_request"], 0.05, "the highest price for the image made");
        assert!(prices["openrouter"]["vendor/flat-image"]["note"].as_str().unwrap().contains("input_text"));
        assert!(prices["openrouter"].get("vendor/token-image").is_none());
        assert!((prices["fal"]["minimax/h3-max/reference-to-video"]["per_minute"].as_f64().unwrap() - 4.8).abs() < 1e-9);
        for (_, models) in prices.as_object().unwrap() {
            for (_, price) in models.as_object().unwrap() {
                assert!(price["from"].is_string() && price["fetched"].is_string(), "{price}");
            }
        }
        // The card says where a cost came from: today's list, or a price
        // written into the spec or step, which is not taken for the list's.
        let listed = json!({ "endpoint": { "kind": "openrouter-image", "model": "vendor/flat-image" }, "estimate": "about $0.05 a request" });
        assert!(note_for(&data, &listed).unwrap().starts_with("is the price list's"));
        let own = json!({ "endpoint": { "kind": "openrouter-image", "model": "vendor/flat-image" }, "estimate": "about $0.01 a request" });
        assert!(note_for(&data, &own).unwrap().contains("own endpoint"), "a spec's own price was shown as the list's");
        let _ = std::fs::remove_dir_all(&data);
    }
}
