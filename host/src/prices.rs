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
    let today = today();
    for (service, models) in read(data).as_object().into_iter().flatten() {
        for (model, entry) in models.as_object().into_iter().flatten() {
            // Only a price read today: one older is read again before use.
            if entry.get("fetched").and_then(Value::as_str) != Some(today.as_str()) {
                continue;
            }
            if let Some(price) = entry.get("price").filter(|price| price.is_object()) {
                let mut price = price.clone();
                price["fetched"] = entry.get("fetched").cloned().unwrap_or(Value::Null);
                prices[service.as_str()][model.as_str()] = price;
            }
        }
    }
    prices
}

/// What a price for the model a plan names leaves out, such as "the image
/// made only", for the card to show beside its cost.
pub(crate) fn note_for(data: &Path, plan: &Value) -> Option<String> {
    let kind = plan.pointer("/endpoint/kind").and_then(Value::as_str)?;
    let service = match kind.split('-').next()? {
        "openrouter" => "openrouter",
        "fal" => "fal",
        _ => return None,
    };
    let model = plan.pointer("/endpoint/model").and_then(Value::as_str)?;
    known(data).get(service)?.get(model)?.get("note")?.as_str().map(String::from)
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
    let price = match fetched {
        Ok(price) => price,
        // The service could not be reached: the last price read is kept,
        // saying when it was read, rather than lost.
        Err(_) => {
            let last = cache.get(service).and_then(|models| models.get(model)).cloned().unwrap_or(Value::Null);
            last.get("price").filter(|price| price.is_object()).map(|price| {
                let mut price = price.clone();
                let read_on = last.get("fetched").and_then(Value::as_str).unwrap_or("an earlier day");
                let note = price.get("note").and_then(Value::as_str).map(|note| format!("{note}; ")).unwrap_or_default();
                price["note"] = json!(format!("{note}read on {read_on}, as today's read failed"));
                price
            })
        }
    };
    // A price not found is kept too, so it is not asked for again today.
    cache[service][model] = json!({ "fetched": today, "price": price });
    let _ = std::fs::write(data.join(CACHE), serde_json::to_string_pretty(&cache).unwrap_or_default());
    price.is_some()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

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
        let _ = std::fs::remove_dir_all(&data);
    }
}
