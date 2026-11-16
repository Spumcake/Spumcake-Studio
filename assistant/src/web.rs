//! The web, for the assistant's research: a search on Brave Search, a
//! page's text, and a file downloaded. The host calls these only for what
//! the person approved, except a page's text, which costs nothing.

use serde_json::{Value, json};
use std::io::Read;
use std::path::Path;
use std::time::Duration;

/// Brave Search's API root, or a stand-in's for tests, from
/// `INPAINTER_BRAVE_URL`. Tests never call Brave.
fn brave_root() -> String {
    std::env::var("INPAINTER_BRAVE_URL").unwrap_or_else(|_| String::from("https://api.search.brave.com/res/v1"))
}

/// The most of a page's text kept, before markup is taken out.
const PAGE_LIMIT: u64 = 4 * 1024 * 1024;

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .timeout_connect(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// One search, of pages or of images, with each result's title, address,
/// and the page it is on.
pub fn brave_search(key: &str, query: &str, images: bool) -> Result<Vec<Value>, String> {
    let kind = if images { "images" } else { "web" };
    let mut response = agent(Duration::from_secs(30))
        .get(&format!("{}/{kind}/search", brave_root()))
        .query("q", query)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", key)
        .call()
        .map_err(|error| format!("Could not reach Brave Search: {error}."))?;
    let status = response.status();
    let text = response.body_mut().read_to_string().map_err(|error| format!("Brave Search's reply could not be read: {error}."))?;
    if !status.is_success() {
        return Err(format!("Brave Search answered {}: {}", status.as_u16(), text.chars().take(300).collect::<String>()));
    }
    let body: Value = serde_json::from_str(&text).map_err(|error| format!("Brave Search's reply was not JSON: {error}."))?;
    let found = if images { body.get("results") } else { body.get("web").and_then(|web| web.get("results")) };
    Ok(found
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .map(|result| {
                    let text = |path: &[&str]| {
                        path.iter().try_fold(result, |at, key| at.get(*key)).and_then(Value::as_str).unwrap_or("").to_string()
                    };
                    if images {
                        json!({
                            "title": text(&["title"]),
                            "image": text(&["properties", "url"]),
                            "page": text(&["url"]),
                            "source": text(&["source"]),
                        })
                    } else {
                        json!({ "title": text(&["title"]), "page": text(&["url"]), "description": text(&["description"]) })
                    }
                })
                .collect()
        })
        .unwrap_or_default())
}

/// A page's readable text: markup, scripts, and styles taken out, and runs
/// of space closed up.
pub fn page_text(url: &str) -> Result<String, String> {
    let mut response = agent(Duration::from_secs(30)).get(url).call().map_err(|error| format!("Could not reach {url}: {error}."))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{url} answered {}.", status.as_u16()));
    }
    let html = response
        .body_mut()
        .with_config()
        .limit(PAGE_LIMIT)
        .read_to_string()
        .map_err(|error| format!("{url} could not be read as text: {error}."))?;
    Ok(readable(&html))
}

/// Text from HTML, without a parser: what is between tags, minus scripts
/// and styles, with entities for the commonest characters read.
fn readable(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let lower = |text: &str| text.to_ascii_lowercase();
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        out.push(' ');
        let after = &rest[open..];
        let tag = lower(&after[..after.len().min(8)]);
        let skip_to = if tag.starts_with("<script") {
            "</script>"
        } else if tag.starts_with("<style") {
            "</style>"
        } else {
            ">"
        };
        match lower(after).find(skip_to) {
            Some(end) => rest = &after[end + skip_to.len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    let out = out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Downloads `url` into `to`, at most `most` bytes. Returns its size.
pub fn download(url: &str, to: &Path, most: u64) -> Result<u64, String> {
    let mut response = agent(Duration::from_secs(300)).get(url).call().map_err(|error| format!("Could not reach {url}: {error}."))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{url} answered {}.", status.as_u16()));
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(most + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{url} could not be read: {error}."))?;
    if bytes.len() as u64 > most {
        return Err(format!("{url} is larger than {} MB, so it was not kept.", most / (1024 * 1024)));
    }
    if let Some(folder) = to.parent() {
        std::fs::create_dir_all(folder).map_err(|error| format!("Could not make {}: {error}.", folder.display()))?;
    }
    std::fs::write(to, &bytes).map_err(|error| format!("Could not write {}: {error}.", to.display()))?;
    Ok(bytes.len() as u64)
}

#[cfg(test)]
mod tests {
    /// Catches a page read with its scripts, styles, or markup, which would
    /// fill the conversation with code.
    #[test]
    fn a_page_reads_as_its_text() {
        let html = "<html><head><style>p{color:red}</style><script>var x = 1;</script></head><body><h1>The 1939 diner</h1><p>Stools &amp; a counter.</p></body></html>";
        assert_eq!(super::readable(html), "The 1939 diner Stools & a counter.");
    }
}
