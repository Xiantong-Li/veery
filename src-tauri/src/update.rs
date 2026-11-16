use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub available: bool,
    pub url: String,
}

struct Source {
    host: &'static str,
    path: &'static str,
    page: &'static str,
}

const SOURCES: &[Source] = &[
    Source {
        host: "gitee.com",
        path: "/api/v5/repos/li-xiantong/veery/releases/latest",
        page: "https://gitee.com/li-xiantong/veery/releases/latest",
    },
    Source {
        host: "api.github.com",
        path: "/repos/Xiantong-Li/veery/releases/latest",
        page: "https://github.com/Xiantong-Li/veery/releases/latest",
    },
];

pub fn check(current: &str) -> Result<UpdateInfo, String> {
    for source in SOURCES {
        let Ok(body) = http_get(source.host, source.path) else {
            continue;
        };
        let Some(latest) = release_tag(&body) else {
            continue;
        };
        return Ok(UpdateInfo {
            available: is_newer(&latest, current),
            current: current.to_string(),
            latest,
            url: source.page.to_string(),
        });
    }
    Err("offline".to_string())
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(next), Some(now)) => next > now,
        _ => false,
    }
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.trim().trim_start_matches('v');
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn release_tag(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    value.get("tag_name")?.as_str().map(str::to_string)
}

fn http_get(host: &str, path: &str) -> Result<String, ()> {
    let (_, body) = crate::net::get(host, path, 262_144)?;
    String::from_utf8(body).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_release_is_detected() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("latest", "0.1.0"));
    }

    #[test]
    fn release_json_has_a_tag() {
        let tag = release_tag(r#"{"tag_name":"v0.1.0","name":"Veery 0.1.0"}"#);
        assert_eq!(tag.as_deref(), Some("v0.1.0"));
        assert!(release_tag("not json").is_none());
    }
}
