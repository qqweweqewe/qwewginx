# examples

feature 1 — echo: `cargo run -p qwewginx -- -c examples/echo.conf` then `curl http://127.0.0.1:9090/`

feature 3 — workers: `cargo run -p qwewginx -- -c examples/workers.conf` then:

```bash
ps -o pid,cmd -C qwewginx   # 1 master + 4 workers
curl http://127.0.0.1:9090/
```

ctrl-c or `kill -TERM <master-pid>` stops all workers.

feature 4 — http/2 (h2c, no tls yet): `cargo run -p qwewginx -- -c examples/h2.conf` then:

```bash
curl --http2-prior-knowledge http://127.0.0.1:9092/
curl --http2 http://127.0.0.1:9092/    # h2c upgrade from http/1.1
curl http://127.0.0.1:9092/            # plain http/1.1 still works
```

alpn h2 over tls lands in feature 5.
