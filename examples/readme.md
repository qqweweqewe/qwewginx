# examples

feature 1 — echo: `cargo run -p qwewginx -- -c examples/echo.conf` then `curl http://127.0.0.1:9090/`

feature 2 — routing: `cargo run -p qwewginx -- -c examples/routing.conf` then:

```bash
curl http://127.0.0.1:9090/          # root
curl http://127.0.0.1:9090/api       # api
curl http://127.0.0.1:9090/api/v1/x  # api v1
curl -i http://127.0.0.1:9090/nope   # 404 not found
```

shutdown: send SIGINT or SIGTERM to the process (ctrl-c in the terminal).
