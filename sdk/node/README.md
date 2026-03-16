# @zeroboot/sdk

Sub-millisecond VM sandboxes for AI agents via copy-on-write forking.

## Usage

```typescript
import { Sandbox } from "./sdk/node/src/index";

const sb = new Sandbox("zb_live_your_api_key");

// Run Python code
const result = await sb.run("import numpy; print(numpy.random.rand(3))");
console.log(result.stdout);        // [0.123 0.456 0.789]
console.log(result.fork_time_ms);  // ~0.75

// Run Node.js code
const jsResult = await sb.run("console.log(JSON.stringify({a: 1}))", {
  language: "node",
});

// Batch execution (runs in parallel)
const results = await sb.runBatch([
  "print(1 + 1)",
  "print(2 * 3)",
  "import math; print(math.pi)",
]);
results.forEach((r) => console.log(r.stdout));
```

## API

### `new Sandbox(apiKey, baseUrl?)`

Create a client. Defaults to `https://api.zeroboot.dev`. Uses the native `fetch` API (Node.js 18+). No external dependencies.

### `sandbox.run(code, options?) -> Promise<Result>`

Execute code in an isolated sandbox.

Options: `{ language?: "python" | "node", timeout?: number }`

### `sandbox.runBatch(codes, options?) -> Promise<Result[]>`

Execute multiple code snippets in parallel sandboxes.

### `Result`

```typescript
{
  id: string;            // UUID request ID
  stdout: string;        // captured output
  stderr: string;        // captured errors
  exit_code: number;     // 0 on success
  fork_time_ms: number;  // VM fork time in milliseconds
  exec_time_ms: number;  // code execution time in milliseconds
  total_time_ms: number; // end-to-end time
}
```
