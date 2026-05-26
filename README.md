# qwewginx

nginx-ish reverse/forward proxy in rust. not suitable for production yet.

loads a small nginx-style config and serves http/1.1 `return` responses. master spawns `worker_processes` workers (reuseport on linux).

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
doc/GUIDE.md     # full spec
doc/ROADMAP.md   # feature order for agents/humans
```

## config

nginx-like dsl. see `examples/echo.conf`, `examples/routing.conf`, `examples/workers.conf`.

