//! Data for the new-tab page's widgets. Fetched here rather than in the UI:
//! github.com sends no CORS headers, and going direct keeps every request
//! between Serica and the source (no third-party proxy sees the username).

use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ContributionDay {
    pub date: String,
    /// 0 (none) to 4 (most), GitHub's own shading level.
    pub level: u8,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Contributions {
    pub user: String,
    pub total: u32,
    /// Oldest first, one entry per day of the last year.
    pub days: Vec<ContributionDay>,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Serica")
        .build()
        .expect("http client")
}

/// GitHub usernames: letters, digits and single hyphens, up to 39 characters.
pub fn valid_github_user(user: &str) -> bool {
    !user.is_empty()
        && user.len() <= 39
        && !user.starts_with('-')
        && !user.ends_with('-')
        && user.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Value of `name="..."` inside one tag's text.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')?;
    Some(&tag[start..start + end])
}

/// Parses the calendar of `github.com/users/<user>/contributions`: one `<td>`
/// per day (`data-date`, `data-level`, `id`) and a `<tool-tip for=id>` with
/// the count ("3 contributions on ..." / "No contributions on ...").
pub fn parse_contributions(html: &str) -> Vec<ContributionDay> {
    let mut counts = std::collections::HashMap::new();
    let mut rest = html;
    while let Some(i) = rest.find("<tool-tip") {
        rest = &rest[i..];
        let Some(close) = rest.find('>') else { break };
        let tag = &rest[..close];
        let body_end = rest[close..].find("</tool-tip>").map(|e| close + e).unwrap_or(close);
        let text = rest[close + 1..body_end].trim();
        if let Some(for_id) = attr(tag, "for") {
            let n: u32 = text.split_whitespace().next().and_then(|w| w.replace(',', "").parse().ok()).unwrap_or(0);
            counts.insert(for_id.to_string(), n);
        }
        rest = &rest[body_end.max(close)..];
    }

    let mut days = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("<td") {
        rest = &rest[i..];
        let Some(close) = rest.find('>') else { break };
        let tag = &rest[..close];
        if let (Some(date), Some(level)) = (attr(tag, "data-date"), attr(tag, "data-level")) {
            let count = attr(tag, "id").and_then(|id| counts.get(id).copied()).unwrap_or(0);
            days.push(ContributionDay { date: date.to_string(), level: level.parse().unwrap_or(0).min(4), count });
        }
        rest = &rest[close..];
    }
    days.sort_by(|a, b| a.date.cmp(&b.date));
    days
}

pub async fn github_contributions(user: &str) -> Result<Contributions, String> {
    let user = user.trim().trim_start_matches('@');
    if !valid_github_user(user) {
        return Err("invalid-user".into());
    }
    let res = client()
        .get(format!("https://github.com/users/{user}/contributions"))
        .header("Accept", "text/html")
        .send()
        .await
        .map_err(|_| "offline".to_string())?;
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("not-found".into());
    }
    if !res.status().is_success() {
        return Err(format!("http-{}", res.status().as_u16()));
    }
    let html = res.text().await.map_err(|_| "offline".to_string())?;
    let days = parse_contributions(&html);
    if days.is_empty() {
        return Err("unreadable".into());
    }
    let total = days.iter().map(|d| d.count).sum();
    Ok(Contributions { user: user.to_string(), total, days })
}

/// First place matching `query` (Open-Meteo geocoding, no key).
pub async fn geocode(query: &str) -> Result<serde_json::Value, String> {
    let q = query.trim();
    if q.is_empty() || q.len() > 80 {
        return Err("invalid-place".into());
    }
    let res: serde_json::Value = client()
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query(&[("name", q), ("count", "1"), ("format", "json")])
        .send()
        .await
        .map_err(|_| "offline".to_string())?
        .json()
        .await
        .map_err(|_| "unreadable".to_string())?;
    let place = res.get("results").and_then(|r| r.get(0)).ok_or("not-found")?;
    Ok(serde_json::json!({
        "name": place.get("name"),
        "country": place.get("country_code"),
        "lat": place.get("latitude"),
        "lon": place.get("longitude"),
    }))
}

/// Current conditions and today's range (Open-Meteo, no key).
pub async fn weather(lat: f64, lon: f64, fahrenheit: bool) -> Result<serde_json::Value, String> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return Err("invalid-place".into());
    }
    let unit = if fahrenheit { "fahrenheit" } else { "celsius" };
    let res: serde_json::Value = client()
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&[
            ("latitude", lat.to_string()),
            ("longitude", lon.to_string()),
            ("current", "temperature_2m,weather_code,is_day".to_string()),
            ("daily", "temperature_2m_max,temperature_2m_min".to_string()),
            ("forecast_days", "1".to_string()),
            ("timezone", "auto".to_string()),
            ("temperature_unit", unit.to_string()),
        ])
        .send()
        .await
        .map_err(|_| "offline".to_string())?
        .json()
        .await
        .map_err(|_| "unreadable".to_string())?;
    let cur = res.get("current").ok_or("unreadable")?;
    let daily = res.get("daily");
    Ok(serde_json::json!({
        "temperature": cur.get("temperature_2m"),
        "code": cur.get("weather_code"),
        "isDay": cur.get("is_day").and_then(|v| v.as_i64()).map(|v| v == 1).unwrap_or(true),
        "max": daily.and_then(|d| d.get("temperature_2m_max")).and_then(|v| v.get(0)),
        "min": daily.and_then(|d| d.get("temperature_2m_min")).and_then(|v| v.get(0)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_calendar_cells_and_tooltips() {
        let html = r#"<table><tr>
<td tabindex="0" data-ix="0" style="width: 10px" data-date="2026-11-24" id="contribution-day-component-0-0" data-level="0" role="gridcell" class="ContributionCalendar-day"></td>
<td tabindex="0" data-ix="1" data-date="2026-11-25" id="contribution-day-component-1-0" data-level="3" class="ContributionCalendar-day"></td>
</tr></table>
<tool-tip id="tooltip-a" for="contribution-day-component-0-0" popover="manual">No contributions on November 24th.</tool-tip>
<tool-tip id="tooltip-b" for="contribution-day-component-1-0" popover="manual">1,204 contributions on November 25th.</tool-tip>"#;
        let days = parse_contributions(html);
        assert_eq!(
            days,
            vec![
                ContributionDay { date: "2026-11-24".into(), level: 0, count: 0 },
                ContributionDay { date: "2026-11-25".into(), level: 3, count: 1204 },
            ]
        );
    }

    #[test]
    fn github_usernames() {
        assert!(valid_github_user("octo-cat"));
        assert!(!valid_github_user("-bad"));
        assert!(!valid_github_user("a/b"));
        assert!(!valid_github_user(""));
    }
}
