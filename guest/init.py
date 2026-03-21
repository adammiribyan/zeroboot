#!/usr/bin/python3 -u
"""Zeroboot Python guest agent — runs as PID 1 inside the VM.

Responsibilities:
1. Mount filesystems, seed initial entropy for imports
2. Import and warm up numpy/pandas for snapshot
3. Handle __ENTROPY__ commands to reseed kernel CRNG + userspace PRNGs
4. Eval/exec user code, write output + ZEROBOOT_DONE marker

Entropy protocol: The host queues __ENTROPY__<hex> into the serial buffer
BEFORE user code. The guest processes it first (FIFO order), reseeds the
kernel CRNG and userspace PRNGs, then processes user code.
"""
import ctypes, os, sys, io, struct, fcntl

RNDADDENTROPY = 0x40085203
RNDRESEEDCRNG = 0x5207
ENTROPY_PREFIX = "__ENTROPY__"

def mount(src, target, fstype):
    try:
        os.makedirs(target, exist_ok=True)
        libc = ctypes.CDLL("libc.musl-x86_64.so.1", use_errno=True)
        libc.mount(src.encode(), target.encode(), fstype.encode(), 0, None)
    except Exception:
        pass

mount("proc", "/proc", "proc")
mount("sysfs", "/sys", "sysfs")
mount("devtmpfs", "/dev", "devtmpfs")

# Seed kernel entropy pool at boot so getrandom() doesn't block during imports.
# This is a deterministic seed — it only needs to unblock the CRNG, not provide
# real entropy. Real per-fork entropy arrives via the __ENTROPY__ serial protocol.
try:
    buf = struct.pack("ii", 256 * 8, 256) + b'\x42' * 256
    fd = os.open("/dev/urandom", os.O_WRONLY)
    fcntl.ioctl(fd, RNDADDENTROPY, buf)
    os.close(fd)
except Exception:
    pass

def hex_to_bytes(h):
    out = bytearray()
    for i in range(0, len(h) - 1, 2):
        out.append(int(h[i:i+2], 16))
    return bytes(out)

def reseed_kernel(seed_bytes):
    fd = os.open("/dev/urandom", os.O_WRONLY)
    try:
        buf = struct.pack("ii", len(seed_bytes) * 8, len(seed_bytes)) + seed_bytes
        fcntl.ioctl(fd, RNDADDENTROPY, buf)
        try:
            fcntl.ioctl(fd, RNDRESEEDCRNG, 0)
        except OSError:
            pass
    finally:
        os.close(fd)

def reseed_userspace():
    fresh = os.urandom(32)
    random.seed(fresh)
    try:
        rs = numpy.random.RandomState(int.from_bytes(fresh[:4], "little"))
        numpy.random.set_state(rs.get_state())
    except Exception:
        pass

def handle_entropy(line):
    hex_str = line[len(ENTROPY_PREFIX):]
    try:
        seed = hex_to_bytes(hex_str)
        if len(seed) < 16:
            return
        reseed_kernel(seed)
        reseed_userspace()
    except Exception:
        pass

# --- Imports and warmup ---

serial = os.open("/dev/ttyS0", os.O_RDWR | os.O_NOCTTY)

def serial_write(msg):
    os.write(serial, msg.encode() if isinstance(msg, str) else msg)

def serial_readline():
    buf = b""
    while True:
        c = os.read(serial, 1)
        if not c:
            continue
        if c in (b"\n", b"\r"):
            if buf:
                return buf.decode("utf-8", errors="replace")
            continue
        buf += c

import numpy, json, math, random, time

try:
    import pandas
except ImportError:
    pandas = None

# Warmup: exercise code paths to pull lazy-loaded modules into the page cache.
# Forked VMs have no disk — any module not already loaded will hang on import.
# This is NOT about SIMD dispatch (AVX2 is masked at the CPUID level).
_ = numpy.array([1.0, 2.0]) + numpy.array([3.0, 4.0])
_ = numpy.random.RandomState(0).rand(1)
_ = numpy.random.default_rng(0).random(1)

if pandas is not None:
    _ = pandas.DataFrame({"a": [1.0]}).describe()

del _
import gc; gc.collect()
time.sleep(0.01)

serial_write("READY\n")

# --- Command loop ---

g = {"__builtins__": __builtins__, "numpy": numpy, "np": numpy,
     "json": json, "math": math, "random": random, "os": os, "sys": sys}
if pandas is not None:
    g["pandas"] = pandas
    g["pd"] = pandas

while True:
    try:
        line = serial_readline()
        if not line:
            continue

        # Entropy reseed: host queues this BEFORE user code (FIFO).
        if line.startswith(ENTROPY_PREFIX):
            handle_entropy(line)
            continue

        old_out, old_err = sys.stdout, sys.stderr
        cap = io.StringIO()
        sys.stdout = sys.stderr = cap
        try:
            c = compile(line, "<input>", "eval")
            r = eval(c, g)
            if r is not None:
                print(r)
        except SyntaxError:
            try:
                exec(line, g)
            except Exception as e:
                print(f"Error: {e}")
        except Exception as e:
            print(f"Error: {e}")
        sys.stdout, sys.stderr = old_out, old_err
        o = cap.getvalue()
        if o:
            serial_write(o)
        serial_write("ZEROBOOT_DONE\n")
    except Exception as e:
        sys.stdout, sys.stderr = sys.__stdout__, sys.__stderr__
        serial_write(f"Agent error: {e}\nZEROBOOT_DONE\n")
