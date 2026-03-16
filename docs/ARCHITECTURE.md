# Architecture

## Overview

```
                    ┌─────────────────────────────────────────┐
                    │          API Server (axum/tokio)        │
                    │  auth · rate-limit · metrics · batch    │
                    └──────────────┬──────────────────────────┘
                                   │
                    ┌──────────────▼──────────────────────────┐
                    │          Fork Engine (kvm.rs)           │
                    │                                         │
                    │  1. KVM create_vm + create_irq_chip     │
                    │  2. Restore IOAPIC redirect table       │
                    │  3. mmap(MAP_PRIVATE) snapshot memory   │
                    │  4. Restore CPU: sregs → XCRS → XSAVE   │
                    │     → regs → LAPIC → MSRs → MP state    │
                    │  5. Serial I/O via 16550 UART emulation │
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

## How It Works

### Template Creation (one-time)

Firecracker boots a VM with your runtime (Python+numpy+pandas, Node.js, etc.), pre-loads modules, and snapshots the full memory + CPU state. This takes ~15 seconds and produces a memory dump and vmstate file.

### Fork (~0.8ms)

Creates a new KVM VM, maps the snapshot memory with `MAP_PRIVATE` (copy-on-write), and restores all CPU state:

1. **KVM VM creation** — `KVM_CREATE_VM` + `KVM_CREATE_IRQCHIP` + `KVM_CREATE_PIT2`
2. **IOAPIC restore** — Read existing irqchip state, overwrite redirect table entries from snapshot, write back (do not zero-init)
3. **Memory mapping** — `mmap(MAP_PRIVATE)` on the snapshot file gives CoW semantics: reads hit the shared snapshot, writes trigger per-fork page faults
4. **CPU state restore** — Must follow exact order: `sregs` → `XCRS` → `XSAVE` → `regs` → `LAPIC` → `MSRs` → `MP_STATE`
5. **Serial I/O** — 16550 UART emulation for guest communication via `/dev/ttyS0`

### Isolation

Each fork gets its own KVM VM with private memory pages. Writes trigger CoW page faults — forks cannot see each other's data. This is hardware-enforced isolation via Intel VT-x/AMD-V, not containers or namespaces.

## Source Layout

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

## Key Implementation Details

### Vmstate Parsing

Firecracker's vmstate is a binary blob with variable-length versionize sections. Offsets shift between rootfs variants and Firecracker versions. The parser auto-detects field locations using the IOAPIC base address (`0xFEC00000`) as an anchor pattern — never hardcode offsets.

### Entropy in Guests

`getrandom()` blocks in Firecracker VMs until the CRNG is initialized. Guest init scripts must seed entropy via the `RNDADDENTROPY` ioctl and pass `random.trust_cpu=on` as a kernel boot argument. The Node.js template uses a Python wrapper as PID 1 to handle entropy seeding before exec'ing node.

### Numpy SIMD Dispatch

Firecracker's CPUID filtering confuses numpy's runtime CPU feature detection. Set `NPY_DISABLE_CPU_FEATURES` in the guest init before importing numpy to avoid SIGILL crashes.

### IOAPIC Restore Pattern

Don't zero-init `kvm_irqchip`. Use `KVM_GET_IRQCHIP` first, then overwrite the redirect table entries from the snapshot, then `KVM_SET_IRQCHIP`. Zero-initializing corrupts other irqchip state and causes interrupt routing failures.
