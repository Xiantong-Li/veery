use serde::Serialize;
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
};

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

struct Internet(*mut core::ffi::c_void);

impl Drop for Internet {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn http_get(host: &str, path: &str) -> Result<String, ()> {
    unsafe {
        let session = WinHttpOpen(
            windows::core::w!("Veery"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            windows::core::PCWSTR(std::ptr::null()),
            windows::core::PCWSTR(std::ptr::null()),
            0,
        );
        if session.is_null() {
            return Err(());
        }
        let session = Internet(session);
        let _ = WinHttpSetTimeouts(session.0, 4000, 4000, 8000, 8000);
        let host_wide: Vec<u16> = host.encode_utf16().chain(std::iter::once(0)).collect();
        let connect = WinHttpConnect(
            session.0,
            windows::core::PCWSTR(host_wide.as_ptr()),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        );
        if connect.is_null() {
            return Err(());
        }
        let connect = Internet(connect);
        let path_wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let request = WinHttpOpenRequest(
            connect.0,
            windows::core::w!("GET"),
            windows::core::PCWSTR(path_wide.as_ptr()),
            windows::core::PCWSTR(std::ptr::null()),
            windows::core::PCWSTR(std::ptr::null()),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        if request.is_null() {
            return Err(());
        }
        let request = Internet(request);
        let headers: Vec<u16> = "User-Agent: Veery\r\nAccept: application/json\r\n"
            .encode_utf16()
            .collect();
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0).map_err(|_| ())?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|_| ())?;
        let mut body = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut read,
            )
            .map_err(|_| ())?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > 262_144 {
                return Err(());
            }
        }
        String::from_utf8(body).map_err(|_| ())
    }
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
