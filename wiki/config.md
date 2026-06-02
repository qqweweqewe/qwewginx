# config

nginx-like dsl. **not full nginx** — if it's not listed here, it doesn't work.

## skeleton

```nginx
worker_processes 1;

events {
    worker_connections 1024;   # parsed, not enforced yet
}

http {
    upstream backend {         # optional
        server 127.0.0.1:9091;
    }

    server {
        listen 127.0.0.1:8080;
        listen 127.0.0.1:443 ssl;   # needs certs below

        ssl_certificate     path/to/cert.pem;
        ssl_certificate_key path/to/key.pem;

        location / {
            return 200 "hello\n";
            # OR: proxy_pass http://backend;
            # OR: root examples/static;
        }
    }
}
```

`#` comments ok. strings in double quotes (`\n` etc work).

---

## directives


| directive                        | where    | notes                                   |
| -------------------------------- | -------- | --------------------------------------- |
| `worker_processes N`             | top      | master spawns N workers (default 1)     |
| `worker_connections N`           | events   | default 1024, not enforced yet          |
| `upstream name { server addr; }` | http     | named backend for proxy_pass            |
| `listen addr`                    | server   | `127.0.0.1:8080` or `:8080` → localhost |
| `listen addr ssl`                | server   | tls + alpn h2/http1.1, needs cert + key |
| `ssl_certificate path`           | server   | pem file                                |
| `ssl_certificate_key path`       | server   | pem file, must pair with cert           |
| `location /path { ... }`         | server   | prefix match, see routing               |
| `return STATUS "body"`           | location | synthetic response, text/plain          |
| `proxy_pass http://...`          | location | reverse proxy, see below                |
| `root path`                      | location | document root for static files          |
| `index file ...`                 | location | index filenames (default `index.html`)  |


unknown directive = parse error.

---

## routing

**longest prefix wins.** config order doesn't matter.


| location  | `/api/v1/x`             | `/apifoo`                       |
| --------- | ----------------------- | ------------------------------- |
| `/`       | matches (fallback)      | matches                         |
| `/api`    | no ( `/api/v1` longer ) | **no** — needs `/` after prefix |
| `/api/v1` | **yes**                 | no                              |


no regex locations, no `=`, no `try_files`.

---

## proxy_pass

```nginx
proxy_pass http://backend;           # upstream name
proxy_pass http://127.0.0.1:9091;    # direct
```

- **http only** — `https://` rejected
- no path suffix on target (`http://backend/foo/` — no)
- forwards full client uri (path + query) as-is, no prefix stripping
- sets `Host` to upstream if client didn't send one
- upstream down / missing name → **502** `bad gateway\n`
- multiple `server` in upstream: **round-robin** among healthy peers, per worker (feature 8)
- passive health (feature 9): connect/timeout errors and upstream **502 / 503 / 504** mark peer down for **10s**, retry next peer in the pool; direct `proxy_pass` has no failover
- all peers down → **502**; other status codes (e.g. app **500**) do not mark peers down

---

## static files

```nginx
location / {
    root examples/static;
    index index.html;
}
```

- **GET/HEAD only** — other methods → **405**
- maps request uri under `root` (longest-prefix location applies)
- directory or missing file → tries `index` files in order
- `..` in path → **403** `forbidden\n`
- missing file → **404** `not found\n`
- content-type from file extension
- paths relative to **cwd** where you start qwewginx (run from repo root for examples)
- no `try_files`, no autoindex, no `alias`

---

## tls dev certs

```bash
sh examples/tls/gen-certs.sh   # writes examples/tls/*.pem (gitignored)
```

browsers/curl need `-k` for self-signed.

ports 80/443 need root or `setcap` — use high ports in dev.

---

## not supported (yet)

`stream {}`, forward proxy, CONNECT, access_log, plugins, config reload, graceful drain.

see [features.md](features.md) for roadmap-ish list.