# Zeroboot Agent Demo

A conversational AI agent that answers questions by writing and executing Python code in Zeroboot sandboxes. Uses Claude tool_use with `run_python` and `run_parallel` tools. Supports single execution and parallel multi-approach comparisons via the batch API.

## Setup

```bash
pip install anthropic requests python-dotenv rich
```

Create `demo/.env`:

```
ANTHROPIC_API_KEY=sk-...
ZEROBOOT_API_KEY=zb_live_...
```

## Run

```bash
python agent.py
```

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `ANTHROPIC_API_KEY` | (required) | Anthropic API key |
| `ZEROBOOT_API_KEY` | (empty) | Zeroboot API key (if auth is enabled) |
| `ZEROBOOT_URL` | `https://api.zeroboot.dev` | Zeroboot API base URL |
| `CLAUDE_MODEL` | `claude-sonnet-4-6` | Claude model to use |
