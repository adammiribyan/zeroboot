# Zeroboot Python SDK

Sub-millisecond code execution sandboxes. Each call runs in an isolated VM forked via copy-on-write memory cloning.

## Usage

```python
import sys
sys.path.insert(0, "sdk/python")

from zeroboot import Sandbox

sb = Sandbox("zb_live_your_api_key")

# Run Python code
result = sb.run("import numpy; print(numpy.random.rand(3))")
print(result.stdout)        # [0.123 0.456 0.789]
print(result.fork_time_ms)  # ~0.75

# Run Node.js code
result = sb.run("console.log(JSON.stringify({a: 1}))", language="node")

# Batch execution (runs in parallel)
results = sb.run_batch([
    "print(1 + 1)",
    "print(2 * 3)",
    "import math; print(math.pi)",
])
for r in results:
    print(r.stdout)
```

## API

### `Sandbox(api_key, base_url="https://api.zeroboot.dev")`

Create a client. No external dependencies required.

### `Sandbox.run(code, language="python", timeout=30) -> Result`

Execute code in an isolated sandbox. Returns a `Result` with:
- `stdout` / `stderr` — captured output
- `exit_code` — 0 on success
- `fork_time_ms` — VM fork time in milliseconds
- `exec_time_ms` — code execution time in milliseconds
- `total_time_ms` — end-to-end time including fork

### `Sandbox.run_batch(codes, language="python", timeout=30) -> list[Result]`

Execute multiple code snippets in parallel sandboxes.

## Zero Dependencies

This SDK uses only Python's standard library (`urllib`). No `requests`, `httpx`, or other packages needed.
