# qwewginx wiki

pet-project nginx-ish proxy in rust. **not production-ready.**

shipped through **feature 10** (https upstream). next: active health checks.

## quick start

```bash
cargo run -p qwewginx -- -c examples/echo.conf
curl http://127.0.0.1:9090/
```

debug parsed config: `--print-ast`  
verbose logs: `-l debug` or `--log-level trace` (default `info`; `RUST_LOG` still works for other crates)

```bash
cargo test
```

## docs in here

- [config.md](config.md) — dsl reference, what's supported
- [features.md](features.md) — what works today, example confs, curl recipes

agent build order lives in `doc/ROADMAP.md` if you have it locally.

## layout (repo)

```
qwewginx/        binary, master/worker cli
qwewginx-core/   parser + server
examples/        *.conf per feature
wiki/            you are here
```

