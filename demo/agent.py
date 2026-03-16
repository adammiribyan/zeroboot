#!/usr/bin/env python3
"""Polished Rich TUI for the Zeroboot parallel agent."""

import os
import time
from pathlib import Path

from dotenv import load_dotenv
import anthropic
from zeroboot import Sandbox
from rich.console import Console
from rich.live import Live
from rich.markdown import Markdown
from rich.panel import Panel
from rich.spinner import Spinner
from rich.table import Table
from rich.text import Text

load_dotenv(Path(__file__).parent / ".env")

MODEL = os.environ.get("CLAUDE_MODEL", "claude-sonnet-4-6")
VERSION = "0.1.0"

sandbox = Sandbox(
    api_key=os.environ.get("ZEROBOOT_API_KEY", ""),
    base_url=os.environ.get("ZEROBOOT_URL", "https://api.zeroboot.dev"),
)
console = Console()

SYSTEM = """\
You solve problems by running Python code. You have two tools:
- run_python: execute a single snippet
- run_parallel: execute multiple approaches in parallel (prefer this for comparisons)
When using run_parallel, provide exactly 5 different approaches with short labels.
ONLY use numpy, pandas, and builtins. Do NOT import statistics or other stdlib modules.
Keep code terse. Always print() results. NEVER call tools more than once per question.
IMPORTANT: The sandbox uses serial I/O. stdout contains echoed code interleaved with \
print() output. If exit_code=0, the code ran successfully. If the output values are \
hard to parse from the echoed code, reason about what the code WOULD produce and give \
your best answer. Never say you need to retry or run again."""

TOOLS = [
    {
        "name": "run_python",
        "description": "Execute Python in a sandbox with numpy and pandas.",
        "input_schema": {
            "type": "object",
            "properties": {"code": {"type": "string"}},
            "required": ["code"],
        },
    },
    {
        "name": "run_parallel",
        "description": "Run multiple Python snippets in parallel sandboxes.",
        "input_schema": {
            "type": "object",
            "properties": {
                "approaches": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": {"type": "string"},
                            "code": {"type": "string"},
                        },
                        "required": ["label", "code"],
                    },
                }
            },
            "required": ["approaches"],
        },
    },
]

def exec_single(code):
    t0 = time.perf_counter()
    try:
        result = sandbox.run(code)
        wall = (time.perf_counter() - t0) * 1000
        return result, wall
    except Exception as e:
        wall = (time.perf_counter() - t0) * 1000
        console.print(f"  [red]API error:[/] {e}")
        from zeroboot import Result
        return Result(stdout="", stderr=str(e), exit_code=1), wall


def exec_batch(approaches):
    t0 = time.perf_counter()
    try:
        results = sandbox.run_batch([a["code"] for a in approaches])
        wall = (time.perf_counter() - t0) * 1000
        return results, wall
    except Exception as e:
        wall = (time.perf_counter() - t0) * 1000
        console.print(f"  [red]API error:[/] {e}")
        from zeroboot import Result
        return [Result(stdout="", stderr=str(e), exit_code=1)
                for _ in approaches], wall


def results_table(approaches, results, wall_ms):
    exec_ms = max(r.total_time_ms for r in results)
    n = len(approaches)

    table = Table(title=f"[bold green]{n} VMs forked and executed in {exec_ms:.0f}ms[/]",
                  title_style="", border_style="dim", pad_edge=False, expand=True)
    table.add_column("Approach", style="cyan", min_width=20)
    table.add_column("Exec", justify="right", style="white", min_width=8)
    table.add_column("Fork", justify="right", style="dim", min_width=8)
    table.add_column("Status", justify="center", min_width=6)

    for a, r in zip(approaches, results):
        if r.exit_code != 0:
            status = "[red]error[/]"
        else:
            status = "[green]done[/]"
        table.add_row(
            a["label"],
            f"{r.exec_time_ms:.1f}ms",
            f"{r.fork_time_ms:.1f}ms",
            status,
        )

    return table


def fmt_time(ms):
    return f"{ms/1000:.1f}s" if ms >= 1000 else f"{ms:.0f}ms"


def format_tool_result(approaches, results, wall_ms):
    lines = []
    for a, r in zip(approaches, results):
        if r.exit_code != 0:
            lines.append(f"[{a['label']}] FAILED (exit {r.exit_code}): {r.stderr.strip()}")
        else:
            lines.append(f"[{a['label']}] exit_code=0, {r.total_time_ms:.1f}ms, raw stdout:\n{r.stdout.strip()}")
    exec_ms = max(r.total_time_ms for r in results)
    lines.append(f"\nAll {len(approaches)} sandboxes executed in {exec_ms:.0f}ms (parallel)")
    lines.append("All code with exit_code=0 ran successfully. Interpret the results and give your answer.")
    return "\n".join(lines)


def llm_call(client, messages, tools=None):
    kwargs = dict(model=MODEL, max_tokens=4096, system=SYSTEM, messages=messages)
    if tools:
        kwargs["tools"] = tools
    return client.messages.create(**kwargs)


def show_header():
    console.print(f"[bold]Zeroboot Agent[/] [dim]v{VERSION} | {MODEL} | https://zeroboot.dev[/]")
    console.rule(style="dim")


def handle_single(block, messages):
    code = block.input["code"]
    console.print(Panel(code, title="[bold]Executing...[/]", border_style="blue",
                        subtitle="[dim]1 sandbox[/]", expand=False))
    with Live(Spinner("dots", text="Running in sandbox..."), console=console, transient=True):
        result, wall_ms = exec_single(code)

    exec_ms = result.total_time_ms
    status = "[green]done[/]" if result.exit_code == 0 else "[red]error[/]"
    console.print(f"  {status} [dim]executed in {fmt_time(exec_ms)}[/]\n")

    if result.exit_code != 0:
        content = f"FAILED: {result.stderr.strip()}"
    else:
        content = f"stdout: {result.stdout.strip()}"
    messages.append({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": block.id, "content": content}
    ]})


def normalize_approaches(raw):
    """Ensure approaches are always [{label, code}] dicts."""
    out = []
    for a in raw:
        if isinstance(a, dict):
            out.append(a)
        elif isinstance(a, str):
            out.append({"label": a.split("\n")[0][:40], "code": a})
    return out


def handle_parallel(block, messages):
    approaches = normalize_approaches(block.input["approaches"])

    labels = "\n".join(f"  {i+1}. {a['label']}" for i, a in enumerate(approaches))
    console.print(Panel(labels, title=f"[bold]Generating {len(approaches)} approaches...[/]",
                        border_style="cyan", expand=False))

    with Live(Spinner("dots", text=f"Executing in parallel across {len(approaches)} isolated VMs..."),
              console=console, transient=True):
        results, wall_ms = exec_batch(approaches)

    console.print(results_table(approaches, results, wall_ms))
    console.print("")

    messages.append({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": block.id,
         "content": format_tool_result(approaches, results, wall_ms)}
    ]})


def chat(client, messages):
    tool_called = False
    while True:
        label = "Interpreting results..." if tool_called else "Thinking..."
        with Live(Spinner("dots", text=f"[dim]{label}[/]"), console=console, transient=True):
            t0 = time.perf_counter()
            resp = llm_call(client, messages, tools=TOOLS)
            llm_ms = (time.perf_counter() - t0) * 1000
        console.print(f"  [dim]LLM: {fmt_time(llm_ms)}[/]")
        messages.append({"role": "assistant", "content": resp.content})

        if resp.stop_reason != "tool_use":
            return "".join(b.text for b in resp.content if hasattr(b, "text"))

        tool_called = True
        for block in resp.content:
            if block.type == "tool_use":
                if block.name == "run_parallel":
                    handle_parallel(block, messages)
                elif block.name == "run_python":
                    handle_single(block, messages)


def main():
    client = anthropic.Anthropic()
    messages = []

    show_header()
    console.print("[dim]Ask me anything — I can run Python with numpy & pandas.[/]\n")

    while True:
        try:
            q = console.input("[bold green]>[/] ").strip()
        except (EOFError, KeyboardInterrupt):
            console.print("\n[dim]Goodbye.[/]")
            break
        if not q or q.lower() in ("quit", "exit"):
            break

        messages.append({"role": "user", "content": q})
        answer = chat(client, messages)
        console.print()
        console.print(Panel(Markdown(answer), border_style="green", expand=True,
                            subtitle="[dim]powered by zeroboot[/]"))
        console.print()


if __name__ == "__main__":
    main()
