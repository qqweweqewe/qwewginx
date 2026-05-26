#!/usr/bin/env sh
# dev self-signed cert for examples/tls.conf — not for production
set -e
dir="$(cd "$(dirname "$0")" && pwd)"
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout "$dir/key.pem" \
  -out "$dir/cert.pem" \
  -days 365 \
  -subj "/CN=localhost"

echo "wrote $dir/cert.pem and $dir/key.pem"
