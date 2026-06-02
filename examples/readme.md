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

feature 5 — tls: generate certs once, then:

```bash
sh examples/tls/gen-certs.sh
cargo run -p qwewginx -- -c examples/tls.conf
curl -k https://127.0.0.1:443/
curl -k --http2 https://127.0.0.1:443/
curl http://127.0.0.1:80/   # same conf, plain server block
```

feature 6 — reverse proxy: start backend, then proxy:

```bash
cargo run -p qwewginx -- -c examples/backend.conf   # terminal 1 — :9091
cargo run -p qwewginx -- -c examples/proxy.conf     # terminal 2 — :9090
curl http://127.0.0.1:9090/                         # backend body via proxy
# stop backend → curl gets 502 bad gateway
```

feature 6 + tls — proxy with tls front door (backend stays plain http):

```bash
sh examples/tls/gen-certs.sh   # once
cargo run -p qwewginx -- -c examples/backend.conf    # terminal 1 — :9091
cargo run -p qwewginx -- -c examples/proxy-tls.conf  # terminal 2 — :9443 (443 needs root/setcap)
curl -k https://127.0.0.1:9443/
curl -k --http2 https://127.0.0.1:9443/
```

feature 7 — static files (run from repo root):

```bash
cargo run -p qwewginx -- -c examples/static.conf # and go check browser after that :D
```

feature 8 — load balancing (two backends):

```bash
cargo run -p qwewginx -- -c examples/backend1.conf   # term 1 — :9091
cargo run -p qwewginx -- -c examples/backend2.conf   # term 2 — :9092
cargo run -p qwewginx -- -c examples/lb.conf         # term 3 — :9090
curl http://127.0.0.1:9090/   # alternates backend1 / backend2
```

feature 9 — passive upstream health (same confs as feature 8):

```bash
# start backend1, backend2, lb.conf as above; then kill backend1
curl http://127.0.0.1:9090/   # still hits backend2
```
