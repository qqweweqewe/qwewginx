# features

what works today + how to try it. one conf per feature in `examples/`.

---

## shipped

| # | what | conf |
|---|------|------|
| 0 | parser, cli, `--print-ast` | — |
| 1 | echo via `return` | `echo.conf` |
| 2 | longest-prefix routing | `routing.conf` |
| 3 | master + workers, reuseport | `workers.conf` |
| 4 | h2c + http/1.1 on same port | `h2.conf` |
| 5 | tls, alpn h2 + http/1.1 | `tls.conf` |
| 6 | reverse proxy | `proxy.conf` + `backend.conf` |
| 7 | static files | `static.conf` |
| 8 | upstream round-robin | `lb.conf` + `backend1.conf` + `backend2.conf` |
| 9 | passive upstream health | `lb.conf` + backends (same as 8) |

---

## curl recipes

**echo** — `:9090`
```bash
cargo run -p qwewginx -- -c examples/echo.conf
curl http://127.0.0.1:9090/
```

**routing** — `:9090`
```bash
cargo run -p qwewginx -- -c examples/routing.conf
curl http://127.0.0.1:9090/api/v1/x   # api v1
curl http://127.0.0.1:9090/api/foo    # api
```

**workers** — `:9090`, 4 workers
```bash
cargo run -p qwewginx -- -c examples/workers.conf
ps -o pid,cmd -C qwewginx
```

**h2** — `:9092`
```bash
cargo run -p qwewginx -- -c examples/h2.conf
curl --http2-prior-knowledge http://127.0.0.1:9092/
curl http://127.0.0.1:9092/    # http/1.1 still fine
```

**tls** — `:443` ssl, `:80` plain
```bash
sh examples/tls/gen-certs.sh
cargo run -p qwewginx -- -c examples/tls.conf
curl -k https://127.0.0.1:443/
curl -k --http2 https://127.0.0.1:443/
```

**reverse proxy** — backend `:9091`, proxy `:9090`
```bash
cargo run -p qwewginx -- -c examples/backend.conf   # term 1
cargo run -p qwewginx -- -c examples/proxy.conf     # term 2
curl http://127.0.0.1:9090/
# kill backend → 502
```

**static files** — `:9090`, run from repo root so `root` paths resolve
```bash
cargo run -p qwewginx -- -c examples/static.conf
curl http://127.0.0.1:9090/              # index.html
curl http://127.0.0.1:9090/style.css
curl -I http://127.0.0.1:9090/nope.css   # 404
```

**load balancing** — two backends `:9091` + `:9092`, proxy `:9090`
```bash
cargo run -p qwewginx -- -c examples/backend1.conf   # term 1
cargo run -p qwewginx -- -c examples/backend2.conf   # term 2
cargo run -p qwewginx -- -c examples/lb.conf        # term 3
curl http://127.0.0.1:9090/   # backend1
curl http://127.0.0.1:9090/   # backend2
curl http://127.0.0.1:9090/   # backend1 again
```

**passive upstream health** — same three terminals as load balancing; kill one backend:

```bash
# with lb.conf + both backends running, then kill backend1 (term 1)
curl http://127.0.0.1:9090/   # still 200 from backend2
# restart backend1 — back in rotation after ~10s cooldown
```

ctrl-c or `kill -TERM <master-pid>` stops workers.

---

## how it runs

master parses config, spawns `worker_processes` kids (`--worker`). each worker binds listen sockets with reuseport (linux), runs current-thread tokio + hyper. master doesn't serve http.

shutdown = kill workers. no drain, no reload.

---

## stack (roughly)

tokio, hyper, rustls, pest, socket2, tracing, clap.

---

## next up

| # | what |
|---|------|
| 10 | active upstream health checks |
| 11–12 | forward proxy, HTTP CONNECT |
| 13 | tcp stream tunnel (`stream {}`, l4 relay) |
| 14+ | logs, plugins, wrk polish |

post-mvp: http/3, websocket, reload, mTLS, etc — only if asked.

---

## tests

```bash
cargo test -p qwewginx-core
```

parse tests in `qwewginx-core/tests/parse_*.rs`. no full integration suite yet.
