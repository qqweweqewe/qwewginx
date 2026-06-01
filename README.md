# qwewginx

nginx-ish reverse/forward proxy in rust. not suitable for production yet.

loads a small nginx-style config and serves `return` responses or reverse-proxies to upstreams over http/1.1, h2c, and tls (alpn h2 + http/1.1). master spawns `worker_processes` workers (reuseport on linux).

**docs:** [wiki/](wiki/README.md) — config + features (short)

## quick start

```bash
cargo run -p qwewginx -- -c examples/echo.conf
# other terminal:
curl http://127.0.0.1:9090/
```

debug ast only: `cargo run -p qwewginx -- -c examples/echo.conf --print-ast`

```bash
cargo test
```

## layout

```
qwewginx/        # binary
qwewginx-core/   # config parser + (later) server/proxy stuff
examples/        # sample .conf files
```

## config

nginx-like dsl. see [wiki/config.md](wiki/config.md) and `examples/*.conf` (`tls.conf` needs `sh examples/tls/gen-certs.sh` first).