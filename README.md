<p align="center">
  <img src="assets/logo.svg" alt="zeroboot" width="300">
</p>

<p align="center">
  <strong>Sub-millisecond VM sandboxes via copy-on-write forking</strong>
</p>

<p align="center">
  <a href="https://github.com/zeroboot/zeroboot/stargazers"><img src="https://img.shields.io/github/stars/zeroboot/zeroboot?style=flat&color=yellow" alt="Stars"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-2021_edition-orange" alt="Rust"></a>
  <a href="https://api.zeroboot.dev/v1/health"><img src="https://img.shields.io/endpoint?url=https://api.zeroboot.dev/v1/health-badge&label=api" alt="API Status"></a>
</p>

---

![demo](demo/demo.gif)
*5 isolated VMs forked and executed in 16ms*

## Try it

```bash
curl -X POST https://api.zeroboot.dev/v1/exec \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer zb_demo_hn2026' \
  -d '{"code":"import numpy as np; print(np.random.rand(3))"}'
```

## Benchmarks

| Metric | Zeroboot | E2B | microsandbox | Daytona |
|---|---|---|---|---|
| Spawn latency p50 | **0.79ms** | ~150ms | ~200ms | ~27ms |
| Spawn latency p99 | 1.74ms | ~300ms | ~400ms | ~90ms |
| Memory per sandbox | ~265KB | ~128MB | ~50MB | ~50MB |
| Fork + exec (Python) | **~8ms** | - | - | - |
| 1000 concurrent forks | 815ms | - | - | - |

Each sandbox is a real KVM virtual machine with hardware-enforced memory isolation.

## How it works

```
  Firecracker snapshot ──► mmap(MAP_PRIVATE) ──► KVM VM + restored CPU state
                              (copy-on-write)         (~0.8ms)
```

1. **Template** (one-time): Firecracker boots a VM, pre-loads your runtime (Python+numpy, Node.js, etc.), and snapshots memory + CPU state
2. **Fork** (~0.8ms): Creates a new KVM VM, maps snapshot memory as CoW, restores all CPU state, starts executing
3. **Isolation**: Each fork is a separate KVM VM — writes trigger CoW page faults, forks can't see each other's data

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for full details.

## Quickstart

```bash
cargo build --release

# Create a template (one-time, ~15s)
sudo target/release/zeroboot template guest/vmlinux-fc workdir-python/rootfs.ext4 workdir-python 15 /init.py

# Run code
sudo target/release/zeroboot test-exec workdir-python "print(1+1)"

# Start API server
sudo target/release/zeroboot serve "python:workdir-python,node:workdir-node" 8080
```

See [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) for systemd, fleet deploy, and production setup.

## SDKs

**Python** &mdash; [sdk/python](sdk/python/)

```python
from zeroboot import Sandbox
sb = Sandbox("zb_live_your_key", base_url="http://localhost:8080")
result = sb.run("import numpy; print(numpy.random.rand(3))")
```

**TypeScript** &mdash; [sdk/node](sdk/node/)

```typescript
import { Sandbox } from "@zeroboot/sdk";
const sb = new Sandbox("zb_live_your_key", "http://localhost:8080");
const result = await sb.run("console.log(1 + 1)", { language: "node" });
```

## API

See [docs/API.md](docs/API.md) for the full reference.

```
POST /v1/exec        — Execute code in an isolated sandbox
POST /v1/exec/batch  — Parallel batch execution
GET  /v1/health      — Template status
GET  /v1/metrics     — Prometheus metrics
```

## Status

Zeroboot is a working prototype. The fork primitive and benchmarks are real, the API works, but this is not production-hardened yet. If you're interested in using this or contributing, [open an issue](https://github.com/zeroboot/zeroboot/issues).

## License

[Apache-2.0](LICENSE)
