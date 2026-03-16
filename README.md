# Zeroboot

Sub-millisecond VM sandbox engine. Spawns isolated code execution environments in **<1ms** by forking Firecracker snapshots via KVM copy-on-write memory cloning.

![demo](demo/demo.gif)
*5 isolated VMs forked and executed in 16ms*

## Try it

Run this from your terminal. Each request forks a VM in <1ms.

```bash
curl -X POST https://api.zeroboot.dev/v1/exec \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer zb_demo_hn2026' \
  -d '{"code":"import numpy as np; print(np.random.rand(3))"}'
```

## Status

Zeroboot is a working prototype. The fork primitive and benchmarks are real, the API works, but this is not production-hardened yet. Auth is file-based API keys, there's no TLS termination built in, and the vmstate parser is validated against one Firecracker/kernel version. If you're interested in using this in production or contributing, open an issue.

## Benchmarks

| Metric | Zeroboot | E2B | microsandbox | Daytona |
|---|---|---|---|---|
| Spawn latency p50 | **0.79ms** | ~150ms | ~200ms | ~27ms |
| Spawn latency p99 | 1.74ms | ~300ms | ~400ms | ~90ms |
| Memory per sandbox | ~265KB | ~128MB | ~50MB | ~50MB |
| Fork + exec (Python) | **~8ms** | - | - | - |
| 1000 concurrent forks | 815ms | - | - | - |

**32x faster than Daytona. Sub-millisecond spawn achieved.**


Each sandbox is a real KVM virtual machine with memory isolation verified — no containers, no shared kernels.

## Quickstart

### Prerequisites

- Linux with KVM access (`/dev/kvm`)
- [Firecracker](https://github.com/firecracker-microvm/firecracker) v1.12+ at `/usr/local/bin/firecracker`
- Rust toolchain
- A Linux kernel binary (`vmlinux`) and ext4 rootfs image with your runtime (Python, Node.js, etc.)

### Build

```bash
cargo build --release
```

### Create a Template

Templates boot a Firecracker VM, pre-load your runtime, and snapshot it. This is a one-time cost (~15s).

```bash
# Python template (numpy + pandas pre-loaded)
sudo target/release/zeroboot template guest/vmlinux-fc workdir-python/rootfs.ext4 workdir-python 15 /init.py

# Node.js template
sudo target/release/zeroboot template guest/vmlinux-fc workdir-node/rootfs.ext4 workdir-node 10 /init-node.sh
```

### Run Code

```bash
# Direct execution
sudo target/release/zeroboot test-exec workdir-python "print(numpy.random.rand(3))"

# Start API server
sudo target/release/zeroboot serve "python:workdir-python,node:workdir-node" 8080
```

### API

```bash
# Execute Python
curl -X POST localhost:8080/v1/exec \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer zb_live_...' \
  -d '{"code": "import numpy; print(numpy.random.rand(3))", "language": "python"}'

# Execute Node.js
curl -X POST localhost:8080/v1/exec \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer zb_live_...' \
  -d '{"code": "console.log(JSON.stringify({a: 1, b: [2,3]}))", "language": "node"}'
```

Response:
```json
{
  "id": "019cf684-1fd5-73c0-9299-52253f9aa79c",
  "stdout": "[0.89294123 0.64994302 0.71755142]",
  "stderr": "",
  "exit_code": 0,
  "fork_time_ms": 0.75,
  "exec_time_ms": 7.2,
  "total_time_ms": 8.0
}
```

## API Reference

| Endpoint | Method | Description |
|---|---|---|
| `/v1/exec` | POST | Execute code in an isolated sandbox |
| `/v1/exec/batch` | POST | Execute multiple snippets in parallel |
| `/v1/health` | GET | Template status and readiness |
| `/v1/metrics` | GET | Prometheus-format metrics |

### POST /v1/exec

```json
{
  "code": "print(1 + 1)",
  "language": "python",
  "timeout_seconds": 30
}
```

Languages: `python` (default), `node`/`javascript`

### POST /v1/exec/batch

```json
{
  "executions": [
    {"code": "print(1)", "language": "python"},
    {"code": "console.log(2)", "language": "node"}
  ]
}
```

### Authentication

Place API keys in `api_keys.json` (or set `ZEROBOOT_API_KEYS_FILE`):

```json
["zb_live_key1", "zb_live_key2"]
```

If no keys file exists, auth is disabled. Invalid/missing keys return HTTP 401. Rate limited at 100 req/s per key (HTTP 429).

## SDKs

### Python

```bash
pip install ./sdk/python
```

```python
from zeroboot import Sandbox

sb = Sandbox("zb_live_your_key", base_url="http://localhost:8080")
result = sb.run("import numpy; print(numpy.random.rand(3))")
print(result.stdout)        # [0.123 0.456 0.789]
print(result.fork_time_ms)  # ~0.75

# Batch (parallel)
results = sb.run_batch(["print(i)" for i in range(10)])
```

### TypeScript / JavaScript

```bash
npm install ./sdk/node
```

```typescript
import { Sandbox } from "@zeroboot/sdk";

const sb = new Sandbox("zb_live_your_key", "http://localhost:8080");
const result = await sb.run("console.log(1 + 1)", { language: "node" });
console.log(result.stdout); // 2

// Batch (parallel)
const results = await sb.runBatch(["print(1)", "print(2)"]);
```

## Architecture

```
                    ┌─────────────────────────────────────────┐
                    │          API Server (axum/tokio)         │
                    │  auth · rate-limit · metrics · batch     │
                    └──────────────┬──────────────────────────┘
                                   │
                    ┌──────────────▼──────────────────────────┐
                    │          Fork Engine (kvm.rs)            │
                    │                                          │
                    │  1. KVM create_vm + create_irq_chip      │
                    │  2. Restore IOAPIC redirect table        │
                    │  3. mmap(MAP_PRIVATE) snapshot memory    │
                    │  4. Restore CPU: sregs → XCRS → XSAVE   │
                    │     → regs → LAPIC → MSRs → MP state    │
                    │  5. Serial I/O via 16550 UART emulation  │
                    └──────────────┬──────────────────────────┘
                                   │
               ┌───────────────────┼───────────────────┐
               ▼                   ▼                   ▼
         ┌──────────┐       ┌──────────┐       ┌──────────┐
         │  Fork A  │       │  Fork B  │       │  Fork C  │
         │  256MB   │       │  256MB   │       │  256MB   │
         │  (CoW)   │       │  (CoW)   │       │  (CoW)   │
         └──────────┘       └──────────┘       └──────────┘
         Actual RSS:         Actual RSS:         Actual RSS:
           ~265KB              ~265KB              ~265KB
```

**Template creation** (one-time): Firecracker boots a VM with your runtime (Python+numpy+pandas, Node.js, etc.), pre-loads modules, and snapshots the full memory + CPU state.

**Fork** (~0.8ms): Creates a new KVM VM, maps the snapshot memory with `MAP_PRIVATE` (copy-on-write), restores all CPU state (registers, FPU/XSAVE, interrupt routing, clock), and starts executing.

**Isolation**: Each fork gets its own KVM VM with private memory pages. Writes trigger CoW page faults — forks cannot see each other's data.

### Source Layout

| File | Purpose |
|---|---|
| `src/vmm/kvm.rs` | Fork engine: KVM VM + CoW mmap + CPU state restore |
| `src/vmm/vmstate.rs` | Firecracker vmstate parser with auto-detect offsets |
| `src/vmm/firecracker.rs` | Template creation via Firecracker API |
| `src/vmm/serial.rs` | 16550 UART emulation for guest I/O |
| `src/api/handlers.rs` | HTTP API: exec, batch, health, metrics, auth |
| `src/main.rs` | CLI: template, test-exec, bench, serve |
| `sdk/python/` | Python SDK (zero dependencies) |
| `sdk/node/` | TypeScript SDK (zero dependencies, uses fetch) |
| `deploy/` | systemd service + fleet deploy script |

## Deployment

### systemd

```bash
sudo cp deploy/zeroboot.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now zeroboot
```

### Fleet Deploy

```bash
SERVERS="host1 host2 host3" ./deploy/deploy.sh
```

Copies binary, kernel, rootfs to each server, creates templates, installs the systemd service, and verifies health.

## License

Apache-2.0
