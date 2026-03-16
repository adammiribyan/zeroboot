# Deployment

## Prerequisites

- Linux with KVM access (`/dev/kvm`)
- [Firecracker](https://github.com/firecracker-microvm/firecracker) v1.12+ at `/usr/local/bin/firecracker`
- Rust toolchain (for building from source)
- A Linux kernel binary (`vmlinux`) and ext4 rootfs image with your runtime

## Build

```bash
cargo build --release
```

## Create Templates

Templates boot a Firecracker VM, pre-load your runtime, and snapshot it. This is a one-time cost (~15s).

```bash
# Python template (numpy + pandas pre-loaded)
sudo target/release/zeroboot template guest/vmlinux-fc workdir-python/rootfs.ext4 workdir-python 15 /init.py

# Node.js template
sudo target/release/zeroboot template guest/vmlinux-fc workdir-node/rootfs.ext4 workdir-node 10 /init-node.sh
```

> **Note:** Firecracker modifies the rootfs during boot. Always `cp` a fresh rootfs before running `template`.

## Start the API Server

```bash
# Single language
sudo target/release/zeroboot serve workdir-python 8080

# Multi-language
sudo target/release/zeroboot serve "python:workdir-python,node:workdir-node" 8080
```

## Verify

```bash
curl -s localhost:8080/v1/health
```

## systemd Service

Install the provided systemd unit:

```bash
sudo cp deploy/zeroboot.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now zeroboot
```

Check status:

```bash
sudo systemctl status zeroboot
sudo journalctl -u zeroboot -f
```

## Fleet Deploy

Deploy to multiple servers at once:

```bash
SERVERS="host1 host2 host3" ./deploy/deploy.sh
```

This copies the binary, kernel, and rootfs to each server, creates templates, installs the systemd service, and verifies health.

## API Keys

Place a JSON array of key strings in `api_keys.json` (or set `ZEROBOOT_API_KEYS_FILE`):

```json
["zb_live_key1", "zb_live_key2"]
```

If no keys file exists, auth is disabled. See [API docs](API.md) for details.

## TLS

Zeroboot does not include built-in TLS termination. Use a reverse proxy (nginx, Caddy, etc.) in front of the API server for HTTPS.
