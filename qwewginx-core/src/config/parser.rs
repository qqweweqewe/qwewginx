use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use pest::Parser;
use pest_derive::Parser;

use super::ast::*;
use super::error::ConfigError;

#[derive(Parser)]
#[grammar = "nginx.pest"]
pub struct NginxParser;

pub fn parse_file(path: &Path) -> Result<Config, ConfigError> {
    let src = std::fs::read_to_string(path)?;
    parse_str(&src)
}

pub fn parse_str(src: &str) -> Result<Config, ConfigError> {
    let mut pairs = NginxParser::parse(Rule::file, src).map_err(|e| ConfigError::Parse(Box::new(e)))?;
    let file = pairs.next().unwrap();

    let mut worker_processes = 1u32;
    let mut events = Events {
        worker_connections: 1024,
    };
    let mut http = Http {
        upstreams: Vec::new(),
        servers: Vec::new(),
    };

    for stmt in file.into_inner().filter(|p| p.as_rule() == Rule::statement) {
        let item = stmt.into_inner().next().unwrap();
        match item.as_rule() {
            Rule::directive => {
                let mut inner = item.into_inner();
                let name = inner.next().unwrap().as_str();
                let args: Vec<String> = inner.map(arg_to_string).collect();
                apply_toplevel(&mut worker_processes, &mut events, &mut http, name, &args)?;
            }
            Rule::block => {
                let mut inner = item.into_inner();
                let name = inner.next().unwrap().as_str();
                let (_open, stmts) = split_block_open(inner)?;
                match name {
                    "events" => events = parse_events_block(stmts)?,
                    "http" => http = parse_http_block(stmts)?,
                    other => {
                        return Err(ConfigError::Msg(format!(
                            "unknown top-level block: {other}"
                        )));
                    }
                }
            }
            _ => {}
        }
    }

    if http.servers.is_empty() {
        return Err(ConfigError::Msg("http { } needs at least one server".into()));
    }

    Ok(Config {
        worker_processes,
        events,
        http,
    })
}

fn apply_toplevel(
    worker_processes: &mut u32,
    _events: &mut Events,
    _http: &mut Http,
    name: &str,
    args: &[String],
) -> Result<(), ConfigError> {
    match name {
        "worker_processes" => {
            *worker_processes = one_u32(args, "worker_processes")?;
        }
        other => {
            return Err(ConfigError::Msg(format!(
                "unknown top-level directive: {other}"
            )));
        }
    }
    Ok(())
}

fn parse_events_block(stmts: Vec<pest::iterators::Pair<'_, Rule>>) -> Result<Events, ConfigError> {
    let mut worker_connections = 1024u32;
    for stmt in stmts {
        if stmt.as_rule() != Rule::statement {
            continue;
        }
        let d = stmt.into_inner().next().unwrap();
        if let Rule::directive = d.as_rule() {
            let mut inner = d.into_inner();
            let name = inner.next().unwrap().as_str();
            let args: Vec<String> = inner.map(arg_to_string).collect();
            if name == "worker_connections" {
                worker_connections = one_u32(&args, "worker_connections")?;
            } else {
                return Err(ConfigError::Msg(format!(
                    "unknown events directive: {name}"
                )));
            }
        }
    }
    Ok(Events {
        worker_connections,
    })
}

fn parse_http_block(stmts: Vec<pest::iterators::Pair<'_, Rule>>) -> Result<Http, ConfigError> {
    let mut upstreams = Vec::new();
    let mut servers = Vec::new();
    for stmt in stmts {
        if stmt.as_rule() != Rule::statement {
            continue;
        }
        let item = stmt.into_inner().next().unwrap();
        if let Rule::block = item.as_rule() {
            let mut inner = item.into_inner();
            let name = inner.next().unwrap().as_str();
            let (block_name, inner_stmts) = split_block_open(inner)?;
            match name {
                "server" => {
                    servers.push(parse_server_block(inner_stmts)?);
                }
                "upstream" => {
                    let upstream_name = block_name.ok_or_else(|| {
                        ConfigError::Msg("upstream needs a name".into())
                    })?;
                    if upstreams.iter().any(|u: &Upstream| u.name == upstream_name) {
                        return Err(ConfigError::Msg(format!(
                            "duplicate upstream: {upstream_name}"
                        )));
                    }
                    upstreams.push(parse_upstream_block(upstream_name, inner_stmts)?);
                }
                other => {
                    return Err(ConfigError::Msg(format!("unknown http block: {other}")));
                }
            }
        }
    }
    Ok(Http { upstreams, servers })
}

fn parse_upstream_block(
    name: String,
    stmts: Vec<pest::iterators::Pair<'_, Rule>>,
) -> Result<Upstream, ConfigError> {
    let mut servers = Vec::new();
    for stmt in stmts {
        if stmt.as_rule() != Rule::statement {
            continue;
        }
        let d = stmt.into_inner().next().unwrap();
        if let Rule::directive = d.as_rule() {
            let mut inner = d.into_inner();
            let dir = inner.next().unwrap().as_str();
            let args: Vec<String> = inner.map(arg_to_string).collect();
            if dir == "server" {
                let addr = args
                    .first()
                    .ok_or_else(|| ConfigError::Msg("server needs an address".into()))?;
                servers.push(parse_listen_addr(addr)?);
            } else {
                return Err(ConfigError::Msg(format!(
                    "unknown upstream directive: {dir}"
                )));
            }
        }
    }
    if servers.is_empty() {
        return Err(ConfigError::Msg(format!("upstream {name} needs server")));
    }
    Ok(Upstream { name, servers })
}

fn parse_server_block(stmts: Vec<pest::iterators::Pair<'_, Rule>>) -> Result<Server, ConfigError> {
    let mut listeners = Vec::new();
    let mut cert_path = None;
    let mut key_path = None;
    let mut locations = Vec::new();

    for stmt in stmts {
        if stmt.as_rule() != Rule::statement {
            continue;
        }
        let item = stmt.into_inner().next().unwrap();
        match item.as_rule() {
            Rule::directive => {
                let mut inner = item.into_inner();
                let name = inner.next().unwrap().as_str();
                let args: Vec<String> = inner.map(arg_to_string).collect();
                match name {
                    "listen" => {
                        if args.is_empty() {
                            return Err(ConfigError::Msg("listen needs an address".into()));
                        }
                        listeners.push(parse_listen_args(&args)?);
                    }
                    "ssl_certificate" => {
                        cert_path = Some(PathBuf::from(one_string(&args, "ssl_certificate")?));
                    }
                    "ssl_certificate_key" => {
                        key_path = Some(PathBuf::from(one_string(&args, "ssl_certificate_key")?));
                    }
                    other => {
                        return Err(ConfigError::Msg(format!(
                            "unknown server directive: {other}"
                        )));
                    }
                }
            }
            Rule::block => {
                let mut inner = item.into_inner();
                let name = inner.next().unwrap().as_str();
                if name != "location" {
                    return Err(ConfigError::Msg(format!("unknown server block: {name}")));
                }
                let (path, inner_stmts) = split_location_open(inner)?;
                locations.push(parse_location_block(path, inner_stmts)?);
            }
            _ => {}
        }
    }

    if listeners.is_empty() {
        return Err(ConfigError::Msg("server needs listen".into()));
    }

    let tls = match (cert_path, key_path) {
        (Some(cert), Some(key)) => Some(TlsFiles { cert, key }),
        (None, None) => None,
        _ => {
            return Err(ConfigError::Msg(
                "ssl_certificate and ssl_certificate_key must both be set".into(),
            ));
        }
    };

    if listeners.iter().any(|l| l.ssl) && tls.is_none() {
        return Err(ConfigError::Msg(
            "ssl listen needs ssl_certificate and ssl_certificate_key".into(),
        ));
    }

    Ok(Server {
        listeners,
        tls,
        locations,
    })
}

fn parse_location_block(
    path: String,
    stmts: Vec<pest::iterators::Pair<'_, Rule>>,
) -> Result<Location, ConfigError> {
    let mut ret = None;
    let mut proxy_pass = None;
    let mut root = None;
    let mut index = Vec::new();
    for stmt in stmts {
        if stmt.as_rule() != Rule::statement {
            continue;
        }
        let d = stmt.into_inner().next().unwrap();
        if let Rule::directive = d.as_rule() {
            let mut inner = d.into_inner();
            let name = inner.next().unwrap().as_str();
            let args: Vec<String> = inner.map(arg_to_string).collect();
            match name {
                "return" => {
                    if ret.is_some() || proxy_pass.is_some() || root.is_some() {
                        return Err(ConfigError::Msg(format!(
                            "location {path} has duplicate action"
                        )));
                    }
                    if args.len() < 2 {
                        return Err(ConfigError::Msg(
                            "return needs status and body string".into(),
                        ));
                    }
                    let status: u16 = args[0]
                        .parse()
                        .map_err(|_| ConfigError::Msg("return status must be a number".into()))?;
                    let body = args[1].clone();
                    ret = Some(ReturnDirective { status, body });
                }
                "proxy_pass" => {
                    if ret.is_some() || proxy_pass.is_some() || root.is_some() {
                        return Err(ConfigError::Msg(format!(
                            "location {path} has duplicate action"
                        )));
                    }
                    let url = one_string(&args, "proxy_pass")?;
                    proxy_pass = Some(parse_proxy_pass(&url)?);
                }
                "root" => {
                    if ret.is_some() || proxy_pass.is_some() || root.is_some() {
                        return Err(ConfigError::Msg(format!(
                            "location {path} has duplicate action"
                        )));
                    }
                    root = Some(PathBuf::from(one_string(&args, "root")?));
                }
                "index" => {
                    if root.is_none() && ret.is_none() && proxy_pass.is_none() {
                        return Err(ConfigError::Msg(format!(
                            "location {path}: index requires root"
                        )));
                    }
                    if ret.is_some() || proxy_pass.is_some() {
                        return Err(ConfigError::Msg(format!(
                            "location {path} has duplicate action"
                        )));
                    }
                    if args.is_empty() {
                        return Err(ConfigError::Msg("index needs at least one filename".into()));
                    }
                    index.extend(args);
                }
                other => {
                    return Err(ConfigError::Msg(format!(
                        "unknown location directive: {other}"
                    )));
                }
            }
        }
    }
    let action = match (ret, proxy_pass, root) {
        (Some(r), None, None) => LocationAction::Return(r),
        (None, Some(p), None) => LocationAction::ProxyPass(p),
        (None, None, Some(root_path)) => {
            if index.is_empty() {
                index.push("index.html".into());
            }
            LocationAction::Static(StaticFiles { root: root_path, index })
        }
        (None, None, None) => {
            return Err(ConfigError::Msg(format!(
                "location {path} needs return, proxy_pass, or root"
            )));
        }
        _ => {
            return Err(ConfigError::Msg(format!(
                "location {path} has conflicting directives"
            )));
        }
    };
    Ok(Location { path, action })
}

fn parse_proxy_pass(url: &str) -> Result<ProxyPass, ConfigError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| ConfigError::Msg("proxy_pass only supports http:// in v1".into()))?;
    if rest.is_empty() {
        return Err(ConfigError::Msg("proxy_pass needs a target".into()));
    }
    if rest.contains('/') {
        return Err(ConfigError::Msg(
            "proxy_pass uri path not supported in v1".into(),
        ));
    }
    let target = if rest.contains(':') {
        let addr = parse_listen_addr(rest)?;
        ProxyTarget::Direct(addr)
    } else {
        ProxyTarget::Upstream(rest.to_string())
    };
    Ok(ProxyPass { target })
}

fn parse_listen_args(args: &[String]) -> Result<Listen, ConfigError> {
    let mut ssl = false;
    let mut addr_part = None;
    for arg in args {
        if arg == "ssl" {
            ssl = true;
        } else if addr_part.is_none() {
            addr_part = Some(arg.as_str());
        } else {
            return Err(ConfigError::Msg(format!(
                "unexpected listen argument: {arg}"
            )));
        }
    }
    let addr_part = addr_part.ok_or_else(|| ConfigError::Msg("listen needs an address".into()))?;
    Ok(Listen {
        addr: parse_listen_addr(addr_part)?,
        ssl,
    })
}

fn parse_listen_addr(s: &str) -> Result<SocketAddr, ConfigError> {
    // 127.0.0.1:8080 or :8080
    let addr = if s.starts_with(':') {
        format!("127.0.0.1{s}")
    } else {
        s.to_string()
    };
    addr.parse()
        .map_err(|_| ConfigError::Msg(format!("bad listen address: {s}")))
}

fn split_block_open(
    inner: pest::iterators::Pairs<'_, Rule>,
) -> Result<(Option<String>, Vec<pest::iterators::Pair<'_, Rule>>), ConfigError> {
    let mut it = inner.into_iter();
    let open = it.next().unwrap();
    match open.as_rule() {
        Rule::block_open => {
            let mut oi = open.into_inner();
            let first = oi.next();
            if first.is_none() {
                // bare "{"
                return Ok((None, it.collect()));
            }
            // path before "{"
            let path = arg_to_string(first.unwrap());
            Ok((Some(path), it.collect()))
        }
        _ => Err(ConfigError::Msg("expected block_open".into())),
    }
}

fn split_location_open(
    inner: pest::iterators::Pairs<'_, Rule>,
) -> Result<(String, Vec<pest::iterators::Pair<'_, Rule>>), ConfigError> {
    let (path, stmts) = split_block_open(inner)?;
    path.ok_or_else(|| ConfigError::Msg("location needs a path".into()))
        .map(|p| (p, stmts))
}

fn arg_to_string(pair: pest::iterators::Pair<'_, Rule>) -> String {
    match pair.as_rule() {
        Rule::arg => arg_to_string(pair.into_inner().next().unwrap()),
        Rule::string => unescape_str(pair.into_inner().next().unwrap().as_str()),
        Rule::token => pair.as_str().to_string(),
        _ => pair.as_str().to_string(),
    }
}

fn unescape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn one_u32(args: &[String], name: &str) -> Result<u32, ConfigError> {
    let v = args
        .first()
        .ok_or_else(|| ConfigError::Msg(format!("{name} needs a value")))?;
    v.parse()
        .map_err(|_| ConfigError::Msg(format!("{name} must be a number")))
}

fn one_string(args: &[String], name: &str) -> Result<String, ConfigError> {
    args.first()
        .cloned()
        .ok_or_else(|| ConfigError::Msg(format!("{name} needs a value")))
}
