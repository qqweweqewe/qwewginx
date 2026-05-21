# qwewginx

nginx-ish reverse/forward proxy in rust. not suitable for production yet.

right now it only **parses** a small nginx-style config and prints the ast.

## quick start

```bash
cargo build
./target/debug/qwewginx -c examples/echo.conf
```

you should see a `Config { ... }` dump.

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

nginx-like dsl. see `examples/echo.conf` for what's supported today.

