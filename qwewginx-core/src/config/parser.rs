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
    let mut plugins = Plugins::default();
    let mut events = Events {
        worker_connections: 1024,
    };
    let mut stream = Stream::default();
    let mut http = Http {
        access_log: None,
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
                apply_toplevel(&mut worker_processes, name, &args)?;
            }
            Rule::block => {
                let mut inner = item.into_inner();
                let name = inner.next().unwrap().as_str();
                let (open, stmts) = split_block_open(inner)?;
                match name {
                    "plugins" => {
                        if open != BlockOpen::Bare {
                            return Err(ConfigError::Msg(
                                "plugins { } must not have a block header".into(),
                            ));
                        }
                        plugins = parse_plugins_block(stmts)?;
                    }
                    "events" => events = parse_events_block(stmts)?,
                    "http" => http = parse_http_block(stmts)?,
                    "stream" => stream = parse_stream_block(stmts)?,
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

    if http.servers.is_empty() && stream.servers.is_empty() {
        return Err(ConfigError::Msg(
            "need at least one server in http { } or stream { }".into(),
        ));
    }

    if !http.servers.is_empty() {
        validate_upstream_proxy_schemes(&http)?;
    }

    Ok(Config {
        worker_processes,
        plugins,
        events,
        stream,
        http,
    })
}

fn parse_plugins_block(stmts: Vec<pest::iterators::Pair<'_, Rule>>) -> Result<Plugins, ConfigError> {
    let mut registry = None;
    let mut entries: Vec<PluginEntry> = Vec::new();
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
                if name != "registry" {
                    return Err(ConfigError::Msg(format!(
                        "unknown plugins directive: {name}"
                    )));
                }
                if registry.is_some() {
                    return Err(ConfigError::Msg("duplicate registry directive".into()));
                }
                registry = Some(one_string(&args, "registry")?);
            }
            Rule::block => {
                let mut inner = item.into_inner();
                let plugin_name = inner.next().unwrap().as_str().to_string();
                let (open, inner_stmts) = split_block_open(inner)?;
                let BlockOpen::Named(header) = open else {
                    return Err(ConfigError::Msg(format!(
                        "plugin {plugin_name} needs a version"
                    )));
                };
                if entries.iter().any(|e| e.name == plugin_name) {
                    return Err(ConfigError::Msg(format!(
                        "duplicate plugin entry: {plugin_name}"
                    )));
                }
                entries.push(PluginEntry {
                    name: plugin_name,
                    version: header.primary,
                    source: if header.from_local {
                        PluginSource::Local
                    } else {
                        PluginSource::Curated
                    },
                    directives: parse_plugin_directives(inner_stmts)?,
                });
            }
            _ => {}
        }
    }
    Ok(Plugins { registry, entries })
}

fn parse_plugin_directives(
    stmts: Vec<pest::iterators::Pair<'_, Rule>>,
) -> Result<Vec<PluginDirective>, ConfigError> {
    let mut out = Vec::new();
    for stmt in stmts {
        if stmt.as_rule() != Rule::statement {
            continue;
        }
        let item = stmt.into_inner().next().unwrap();
        if item.as_rule() != Rule::directive {
            return Err(ConfigError::Msg(
                "plugin blocks only allow directives".into(),
            ));
        }
        let mut inner = item.into_inner();
        let name = inner.next().unwrap().as_str().to_string();
        let args: Vec<String> = inner.map(arg_to_string).collect();
        out.push(PluginDirective { name, args });
    }
    Ok(out)
}

fn parse_stream_block(stmts: Vec<pest::iterators::Pair<'_, Rule>>) -> Result<Stream, ConfigError> {
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
                return Err(ConfigError::Msg(format!("unknown stream block: {name}")));
            }
            let (open, inner_stmts) = split_block_open(inner)?;
            if open != BlockOpen::Bare {
                return Err(ConfigError::Msg("stream server { } must be bare".into()));
            }
            servers.push(parse_stream_server_block(inner_stmts)?);
        } else {
            return Err(ConfigError::Msg(
                "stream { } only allows server blocks".into(),
            ));
        }
    }
    if servers.is_empty() {
        return Err(ConfigError::Msg("stream { } needs at least one server".into()));
    }
    Ok(Stream { servers })
}

fn parse_stream_server_block(
    stmts: Vec<pest::iterators::Pair<'_, Rule>>,
) -> Result<StreamServer, ConfigError> {
    let mut listen = None;
    let mut proxy_pass = None;
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
                "listen" => {
                    if args.is_empty() {
                        return Err(ConfigError::Msg("listen needs an address".into()));
                    }
                    if args.len() > 1 {
                        return Err(ConfigError::Msg(
                            "stream listen takes one address (no ssl)".into(),
                        ));
                    }
                    listen = Some(parse_listen_addr(&args[0])?);
                }
                "proxy_pass" => {
                    let addr = one_string(&args, "proxy_pass")?;
                    proxy_pass = Some(parse_listen_addr(&addr)?);
                }
                other => {
                    return Err(ConfigError::Msg(format!(
                        "unknown stream server directive: {other}"
                    )));
                }
            }
        }
    }
    let listen = listen.ok_or_else(|| ConfigError::Msg("stream server needs listen".into()))?;
    let proxy_pass =
        proxy_pass.ok_or_else(|| ConfigError::Msg("stream server needs proxy_pass".into()))?;
    Ok(StreamServer { listen, proxy_pass })
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct UpstreamProxyRef {
    scheme: ProxyScheme,
    ssl_verify: bool,
}

fn validate_upstream_proxy_schemes(http: &Http) -> Result<(), ConfigError> {
    use std::collections::HashMap;

    let mut seen: HashMap<&str, UpstreamProxyRef> = HashMap::new();
    for server in &http.servers {
        for loc in &server.locations {
            let LocationAction::ProxyPass(pass) = &loc.action else {
                continue;
            };
            let ProxyTarget::Upstream(name) = &pass.target else {
                continue;
            };
            let cur = UpstreamProxyRef {
                scheme: pass.scheme,
                ssl_verify: pass.ssl_verify,
            };
            if let Some(prev) = seen.get(name.as_str()) {
                if prev.scheme != cur.scheme {
                    return Err(ConfigError::Msg(format!(
                        "upstream {name}: mixed http:// and https:// in proxy_pass"
                    )));
                }
                if prev.ssl_verify != cur.ssl_verify {
                    return Err(ConfigError::Msg(format!(
                        "upstream {name}: inconsistent proxy_ssl_verify across locations"
                    )));
                }
            } else {
                seen.insert(name.as_str(), cur);
            }
        }
    }
    Ok(())
}

fn apply_toplevel(
    worker_processes: &mut u32,
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
    let mut access_log = None;
    let mut upstreams = Vec::new();
    let mut servers = Vec::new();
    for stmt in stmts {
        if stmt.as_rule() != Rule::statement {
            continue;
        }
        let item = stmt.into_inner().next().unwrap();
        if let Rule::directive = item.as_rule() {
            let mut inner = item.into_inner();
            let name = inner.next().unwrap().as_str();
            let args: Vec<String> = inner.map(arg_to_string).collect();
            match name {
                "access_log" => access_log = Some(parse_access_log_args(&args)?),
                other => {
                    return Err(ConfigError::Msg(format!(
                        "unknown http directive: {other}"
                    )));
                }
            }
            continue;
        }
        if let Rule::block = item.as_rule() {
            let mut inner = item.into_inner();
            let name = inner.next().unwrap().as_str();
            let (open, inner_stmts) = split_block_open(inner)?;
            match name {
                "server" => {
                    if open != BlockOpen::Bare {
                        return Err(ConfigError::Msg("server { } must be bare".into()));
                    }
                    servers.push(parse_server_block(inner_stmts)?);
                }
                "upstream" => {
                    let BlockOpen::Named(header) = open else {
                        return Err(ConfigError::Msg("upstream needs a name".into()));
                    };
                    if header.from_local {
                        return Err(ConfigError::Msg(
                            "upstream name cannot use from local".into(),
                        ));
                    }
                    let upstream_name = header.primary;
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
    Ok(Http {
        access_log,
        upstreams,
        servers,
    })
}

fn parse_access_log_args(args: &[String]) -> Result<AccessLogSetting, ConfigError> {
    let arg = args
        .first()
        .ok_or_else(|| ConfigError::Msg("access_log needs a path or off".into()))?;
    if arg == "off" {
        if args.len() > 1 {
            return Err(ConfigError::Msg("access_log off takes no extra args".into()));
        }
        Ok(AccessLogSetting::Off)
    } else {
        if args.len() > 1 {
            return Err(ConfigError::Msg("access_log takes one path argument".into()));
        }
        Ok(AccessLogSetting::Path(PathBuf::from(arg.as_str())))
    }
}

fn parse_upstream_block(
    name: String,
    stmts: Vec<pest::iterators::Pair<'_, Rule>>,
) -> Result<Upstream, ConfigError> {
    let mut servers = Vec::new();
    let mut health_check = None;
    for stmt in stmts {
        if stmt.as_rule() != Rule::statement {
            continue;
        }
        let d = stmt.into_inner().next().unwrap();
        if let Rule::directive = d.as_rule() {
            let mut inner = d.into_inner();
            let dir = inner.next().unwrap().as_str();
            let args: Vec<String> = inner.map(arg_to_string).collect();
            match dir {
                "server" => {
                    let addr = args
                        .first()
                        .ok_or_else(|| ConfigError::Msg("server needs an address".into()))?;
                    servers.push(parse_listen_addr(addr)?);
                }
                "health_check" => {
                    apply_health_check(&mut health_check, &args)?;
                }
                other => {
                    return Err(ConfigError::Msg(format!(
                        "unknown upstream directive: {other}"
                    )));
                }
            }
        }
    }
    if servers.is_empty() {
        return Err(ConfigError::Msg(format!("upstream {name} needs server")));
    }
    Ok(Upstream {
        name,
        servers,
        health_check,
    })
}

fn apply_health_check(
    health_check: &mut Option<HealthCheck>,
    args: &[String],
) -> Result<(), ConfigError> {
    let hc = health_check.get_or_insert_with(HealthCheck::default);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "interval" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| ConfigError::Msg("health_check interval needs a value".into()))?;
                let n: u32 = v
                    .parse()
                    .map_err(|_| ConfigError::Msg("health_check interval must be a number".into()))?;
                if n == 0 {
                    return Err(ConfigError::Msg("health_check interval must be > 0".into()));
                }
                hc.interval_secs = n;
                i += 1;
            }
            "uri" => {
                i += 1;
                let path = args
                    .get(i)
                    .ok_or_else(|| ConfigError::Msg("health_check uri needs a path".into()))?;
                if !path.starts_with('/') {
                    return Err(ConfigError::Msg("health_check uri must start with /".into()));
                }
                hc.uri = path.clone();
                i += 1;
            }
            other => {
                return Err(ConfigError::Msg(format!(
                    "unknown health_check option: {other}"
                )));
            }
        }
    }
    Ok(())
}

fn parse_bool_arg(args: &[String], directive: &str) -> Result<bool, ConfigError> {
    let v = one_string(args, directive)?;
    match v.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ConfigError::Msg(format!("{directive} must be true or false"))),
    }
}

fn parse_server_block(stmts: Vec<pest::iterators::Pair<'_, Rule>>) -> Result<Server, ConfigError> {
    let mut listeners = Vec::new();
    let mut cert_path = None;
    let mut key_path = None;
    let mut access_log = None;
    let mut forward_proxy = false;
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
                    "access_log" => {
                        access_log = Some(parse_access_log_args(&args)?);
                    }
                    "forward_proxy" => {
                        forward_proxy = parse_bool_arg(&args, "forward_proxy")?;
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

    if forward_proxy && !locations.is_empty() {
        return Err(ConfigError::Msg(
            "forward_proxy server cannot have location blocks".into(),
        ));
    }

    Ok(Server {
        listeners,
        tls,
        access_log,
        forward_proxy,
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
    let mut proxy_ssl_verify = true;
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
                    let mut pass = parse_proxy_pass(&url)?;
                    pass.ssl_verify = proxy_ssl_verify;
                    proxy_pass = Some(pass);
                }
                "proxy_ssl_verify" => {
                    let v = one_string(&args, "proxy_ssl_verify")?;
                    proxy_ssl_verify = match v.as_str() {
                        "off" => false,
                        "on" => true,
                        other => {
                            return Err(ConfigError::Msg(format!(
                                "proxy_ssl_verify: expected on or off, got {other}"
                            )));
                        }
                    };
                    if let Some(pass) = proxy_pass.as_mut() {
                        pass.ssl_verify = proxy_ssl_verify;
                    }
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
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        (ProxyScheme::Https, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (ProxyScheme::Http, r)
    } else {
        return Err(ConfigError::Msg(
            "proxy_pass must start with http:// or https://".into(),
        ));
    };
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
    Ok(ProxyPass {
        scheme,
        ssl_verify: true,
        target,
    })
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockHeader {
    primary: String,
    from_local: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BlockOpen {
    Bare,
    Named(BlockHeader),
}

fn split_block_open(
    inner: pest::iterators::Pairs<'_, Rule>,
) -> Result<(BlockOpen, Vec<pest::iterators::Pair<'_, Rule>>), ConfigError> {
    let mut it = inner.into_iter();
    let open = it.next().unwrap();
    match open.as_rule() {
        Rule::block_open => {
            let mut oi = open.into_inner();
            let first = oi.next();
            if first.is_none() {
                return Ok((BlockOpen::Bare, it.collect()));
            }
            let header_pair = first.unwrap();
            if header_pair.as_rule() == Rule::block_header {
                let mut hi = header_pair.into_inner();
                let primary = arg_to_string(hi.next().unwrap());
                let from_local = hi.any(|p| p.as_rule() == Rule::plugin_source);
                return Ok((
                    BlockOpen::Named(BlockHeader {
                        primary,
                        from_local,
                    }),
                    it.collect(),
                ));
            }
            // legacy single-arg header (should not occur with current grammar)
            Ok((
                BlockOpen::Named(BlockHeader {
                    primary: arg_to_string(header_pair),
                    from_local: false,
                }),
                it.collect(),
            ))
        }
        _ => Err(ConfigError::Msg("expected block_open".into())),
    }
}

fn split_location_open(
    inner: pest::iterators::Pairs<'_, Rule>,
) -> Result<(String, Vec<pest::iterators::Pair<'_, Rule>>), ConfigError> {
    let (open, stmts) = split_block_open(inner)?;
    let BlockOpen::Named(header) = open else {
        return Err(ConfigError::Msg("location needs a path".into()));
    };
    if header.from_local {
        return Err(ConfigError::Msg("location path cannot use from local".into()));
    }
    Ok((header.primary, stmts))
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
