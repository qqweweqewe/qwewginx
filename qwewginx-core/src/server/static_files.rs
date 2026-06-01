use std::convert::Infallible;
use std::path::{Component, Path, PathBuf};

use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::{Method, Response, StatusCode};

use crate::config::StaticFiles;
use super::proxy::ResponseBody;

pub async fn serve(
    method: &Method,
    uri_path: &str,
    location_prefix: &str,
    cfg: &StaticFiles,
) -> Response<ResponseBody> {
    if !matches!(*method, Method::GET | Method::HEAD) {
        return method_not_allowed();
    }

    let rel = match uri_to_relative(location_prefix, uri_path) {
        Some(r) => r,
        None => return not_found(),
    };

    match resolve_file(&cfg.root, &rel, &cfg.index).await {
        Ok(path) => file_response(method, &path).await,
        Err(ResolveError::NotFound) => not_found(),
        Err(ResolveError::Forbidden) => forbidden(),
        Err(ResolveError::Io(e)) => {
            tracing::debug!("static file read failed: {e}");
            not_found()
        }
    }
}

enum ResolveError {
    NotFound,
    Forbidden,
    Io(std::io::Error),
}

async fn resolve_file(
    root: &Path,
    rel: &str,
    index: &[String],
) -> Result<PathBuf, ResolveError> {
    let base = match safe_join(root, rel) {
        Some(p) => p,
        None => return Err(ResolveError::Forbidden),
    };

    let meta = match tokio::fs::metadata(&base).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !rel.ends_with('/') {
                for name in index {
                    let candidate = base.join(name);
                    if tokio::fs::metadata(&candidate).await.is_ok() {
                        return Ok(candidate);
                    }
                }
            }
            return Err(ResolveError::NotFound);
        }
        Err(e) => return Err(ResolveError::Io(e)),
    };

    if meta.is_dir() {
        for name in index {
            let candidate = base.join(name);
            if tokio::fs::metadata(&candidate).await.is_ok() {
                return Ok(candidate);
            }
        }
        return Err(ResolveError::NotFound);
    }

    Ok(base)
}

fn uri_to_relative(location_prefix: &str, uri_path: &str) -> Option<String> {
    if location_prefix == "/" {
        return Some(uri_path.to_string());
    }
    if uri_path == location_prefix {
        return Some("/".to_string());
    }
    if uri_path.len() > location_prefix.len()
        && uri_path.starts_with(location_prefix)
        && uri_path.as_bytes()[location_prefix.len()] == b'/'
    {
        return Some(uri_path[location_prefix.len()..].to_string());
    }
    None
}

fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim_start_matches('/');
    let mut out = root.to_path_buf();
    if rel.is_empty() {
        return Some(out);
    }
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

async fn file_response(method: &Method, path: &Path) -> Response<ResponseBody> {
    let bytes = match tokio::fs::read(path).await {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!("static read {}: {e}", path.display());
            return not_found();
        }
    };
    let content_type = guess_content_type(path);
    let len = bytes.len();
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", content_type);
    if *method == Method::HEAD {
        builder = builder.header("content-length", len.to_string());
        return builder
            .body(
                Full::new(Bytes::new())
                    .map_err(|e: Infallible| match e {})
                    .boxed(),
            )
            .unwrap();
    }
    builder
        .body(
            Full::new(Bytes::from(bytes))
                .map_err(|e: Infallible| match e {})
                .boxed(),
        )
        .unwrap()
}

fn guess_content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("txt") => "text/plain; charset=utf-8",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn not_found() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("content-type", "text/plain")
        .body(
            Full::new(Bytes::from("not found\n"))
                .map_err(|e: Infallible| match e {})
                .boxed(),
        )
        .unwrap()
}

fn forbidden() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("content-type", "text/plain")
        .body(
            Full::new(Bytes::from("forbidden\n"))
                .map_err(|e: Infallible| match e {})
                .boxed(),
        )
        .unwrap()
}

fn method_not_allowed() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .header("content-type", "text/plain")
        .header("allow", "GET, HEAD")
        .body(
            Full::new(Bytes::from("method not allowed\n"))
                .map_err(|e: Infallible| match e {})
                .boxed(),
        )
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_from_root_location() {
        assert_eq!(uri_to_relative("/", "/foo/bar").unwrap(), "/foo/bar");
        assert_eq!(uri_to_relative("/", "/").unwrap(), "/");
    }

    #[test]
    fn relative_from_prefix_location() {
        assert_eq!(uri_to_relative("/assets", "/assets").unwrap(), "/");
        assert_eq!(uri_to_relative("/assets", "/assets/app.js").unwrap(), "/app.js");
    }

    #[test]
    fn safe_join_blocks_traversal() {
        assert!(safe_join(Path::new("/var/www"), "../etc/passwd").is_none());
        assert_eq!(
            safe_join(Path::new("/var/www"), "css/app.css").unwrap(),
            PathBuf::from("/var/www/css/app.css")
        );
    }
}
