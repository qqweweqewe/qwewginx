# examples

feature 1 — echo: `cargo run -p qwewginx -- -c examples/echo.conf` then `curl http://127.0.0.1:9090/`

feature 3 — workers: `cargo run -p qwewginx -- -c examples/workers.conf` then:

```bash
ps -o pid,cmd -C qwewginx   # 1 master + 4 workers
curl http://127.0.0.1:9090/
```

ctrl-c or `kill -TERM <master-pid>` stops all workers.
