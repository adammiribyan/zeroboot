export interface Result {
  id: string;
  stdout: string;
  stderr: string;
  exit_code: number;
  fork_time_ms: number;
  exec_time_ms: number;
  total_time_ms: number;
}

export interface ExecOptions {
  language?: string;
  timeout?: number;
}

export class Sandbox {
  private baseUrl: string;
  private headers: Record<string, string>;

  constructor(apiKey: string, baseUrl: string = "http://api.zeroboot.dev") {
    this.baseUrl = baseUrl.replace(/\/$/, "");
    this.headers = {
      Authorization: `Bearer ${apiKey}`,
      "Content-Type": "application/json",
    };
  }

  async run(code: string, options?: ExecOptions): Promise<Result> {
    const resp = await fetch(`${this.baseUrl}/v1/exec`, {
      method: "POST",
      headers: this.headers,
      body: JSON.stringify({
        code,
        language: options?.language ?? "python",
        timeout_seconds: options?.timeout ?? 30,
      }),
    });
    if (!resp.ok) {
      const err = await resp.json().catch(() => ({ error: resp.statusText }));
      throw new Error(`API error (${resp.status}): ${(err as any).error}`);
    }
    return resp.json() as Promise<Result>;
  }

  async runBatch(
    codes: string[],
    options?: ExecOptions
  ): Promise<Result[]> {
    const resp = await fetch(`${this.baseUrl}/v1/exec/batch`, {
      method: "POST",
      headers: this.headers,
      body: JSON.stringify({
        executions: codes.map((code) => ({
          code,
          language: options?.language ?? "python",
          timeout_seconds: options?.timeout ?? 30,
        })),
      }),
    });
    if (!resp.ok) {
      const err = await resp.json().catch(() => ({ error: resp.statusText }));
      throw new Error(`API error (${resp.status}): ${(err as any).error}`);
    }
    const data = (await resp.json()) as { results: Result[] };
    return data.results;
  }
}
