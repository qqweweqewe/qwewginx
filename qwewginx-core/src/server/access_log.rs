use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use hyper::http::Uri;
use hyper::{Method, Version};

use crate::config::{AccessLogSetting, Http, Server};

use super::proxy::UpstreamMeta;

pub struct AccessLog {
    file: Mutex<File>,
}

impl AccessLog {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    pub fn write_line(&self, line: &str) {
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

pub fn resolve_access_log_path(http: &Http, server: &Server) -> Option<PathBuf> {
    match &server.access_log {
        Some(AccessLogSetting::Off) => None,
        Some(AccessLogSetting::Path(p)) => Some(p.clone()),
        None => match &http.access_log {
            Some(AccessLogSetting::Off) => None,
            Some(AccessLogSetting::Path(p)) => Some(p.clone()),
            None => None,
        },
    }
}

pub struct AccessLogEntry<'a> {
    pub remote_addr: Option<SocketAddr>,
    pub method: &'a Method,
    pub uri: &'a Uri,
    pub version: Version,
    pub status: u16,
    pub body_bytes: u64,
    pub request_time: Instant,
    pub upstream: &'a UpstreamMeta,
}

pub fn write_entry(log: &AccessLog, entry: AccessLogEntry<'_>) {
    let elapsed = entry.request_time.elapsed();
    let line = format_combined(
        entry.remote_addr,
        SystemTime::now(),
        entry.method,
        entry.uri,
        entry.version,
        entry.status,
        entry.body_bytes,
        elapsed.as_secs_f64(),
        entry.upstream,
    );
    log.write_line(&line);
}

pub fn format_combined(
    remote_addr: Option<SocketAddr>,
    time: SystemTime,
    method: &Method,
    uri: &Uri,
    version: Version,
    status: u16,
    body_bytes: u64,
    request_time_secs: f64,
    upstream: &UpstreamMeta,
) -> String {
    let remote = remote_addr
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|| "-".into());
    let request_target = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let request_line = format!(
        "{} {} {}",
        method,
        request_target,
        http_version(version)
    );
    let upstream_field = format_upstream(upstream);
    let upstream_status = upstream
        .upstream_status
        .map(|s| s.to_string())
        .unwrap_or_else(|| "-".into());
    format!(
        "{remote} - - {} \"{request_line}\" {status} {body_bytes} {request_time_secs:.3} upstream={upstream_field} upstream_status={upstream_status}",
        format_log_time(time),
    )
}

fn format_upstream(meta: &UpstreamMeta) -> String {
    match (meta.upstream_name.as_deref(), meta.upstream_addr) {
        (Some(name), Some(addr)) => format!("{name}:{addr}"),
        (None, Some(addr)) => format!("-:{addr}"),
        _ => "-".into(),
    }
}

fn http_version(version: Version) -> &'static str {
    match version {
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_11 => "HTTP/1.1",
        Version::HTTP_2 => "HTTP/2",
        _ => "HTTP/1.1",
    }
}

fn format_log_time(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (year, month, day, hour, min, sec) = unix_to_utc(secs);
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "[{day:02}/{month}/{year}:{hour:02}:{min:02}:{sec:02} +0000]",
        month = MONTHS[month as usize - 1],
    )
}

fn unix_to_utc(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hour = (rem / 3600) as u32;
    let min = ((rem % 3600) / 60) as u32;
    let sec = (rem % 60) as u32;

    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe + era * 400) as u32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 {
        (mp + 3) as u32
    } else {
        (mp - 9) as u32
    };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year, month, day, hour, min, sec)
}

pub fn response_body_bytes(status: u16, content_length: Option<u64>, fallback: Option<usize>) -> u64 {
    if status == 204 || status == 304 {
        return 0;
    }
    content_length
        .or(fallback.map(|n| n as u64))
        .unwrap_or(0)
}

pub fn content_length_from_headers(headers: &hyper::HeaderMap) -> Option<u64> {
    headers
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr};

    #[test]
    fn combined_format_includes_upstream_fields() {
        let uri: Uri = "/api?q=1".parse().unwrap();
        let meta = UpstreamMeta {
            upstream_name: Some("backend".into()),
            upstream_addr: Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 9091))),
            upstream_status: Some(200),
        };
        let line = format_combined(
            Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 54321))),
            UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
            &Method::GET,
            &uri,
            Version::HTTP_11,
            200,
            42,
            0.003,
            &meta,
        );
        assert!(line.starts_with("127.0.0.1 - - ["));
        assert!(line.contains("\"GET /api?q=1 HTTP/1.1\""));
        assert!(line.contains(" 200 42 0.003 "));
        assert!(line.contains("upstream=backend:127.0.0.1:9091"));
        assert!(line.ends_with("upstream_status=200"));
    }

    #[test]
    fn resolve_server_overrides_http() {
        let http = Http {
            access_log: Some(AccessLogSetting::Path("/var/log/http.log".into())),
            upstreams: vec![],
            servers: vec![],
        };
        let server = Server {
            listeners: vec![],
            tls: None,
            access_log: Some(AccessLogSetting::Off),
            forward_proxy: false,
            locations: vec![],
        };
        assert!(resolve_access_log_path(&http, &server).is_none());
    }
}
