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
    let mut servers = Vec::new();
    for stmt in stmts {
        if stmt.as_rule() != Rule::statement {
            continue;
        }
        let item = stmt.into_inner().next().unwrap();
        if let Rule::block = item.as_rule() {
            let mut inner = item.into_inner();
            let name = inner.next().unwrap().as_str();
            if name != "server" {
                return Err(ConfigError::Msg(format!("unknown http block: {name}")));
            }
            let (_open, inner_stmts) = split_block_open(inner)?;
            servers.push(parse_server_block(inner_stmts)?);
        }
    }
    Ok(Http { servers })
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
    for stmt in stmts {
        if stmt.as_rule() != Rule::statement {
            continue;
        }
        let d = stmt.into_inner().next().unwrap();
        if let Rule::directive = d.as_rule() {
            let mut inner = d.into_inner();
            let name = inner.next().unwrap().as_str();
            let args: Vec<String> = inner.map(arg_to_string).collect();
            if name == "return" {
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
            } else {
                return Err(ConfigError::Msg(format!(
                    "unknown location directive: {name}"
                )));
            }
        }
    }
    let ret = ret.ok_or_else(|| ConfigError::Msg(format!("location {path} needs return")))?;
    Ok(Location { path, ret })
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
