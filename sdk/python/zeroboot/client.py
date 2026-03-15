"""Zeroboot Python SDK — sub-millisecond code execution sandboxes."""

from dataclasses import dataclass
from typing import Optional
import json
import urllib.request
import urllib.error


@dataclass
class Result:
    id: str = ""
    stdout: str = ""
    stderr: str = ""
    exit_code: int = 0
    fork_time_ms: float = 0.0
    exec_time_ms: float = 0.0
    total_time_ms: float = 0.0


class Sandbox:
    """Client for the Zeroboot sandbox API."""

    def __init__(self, api_key: str, base_url: str = "http://api.zeroboot.dev"):
        self.base_url = base_url.rstrip("/")
        self.headers = {
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        }

    def _request(self, path: str, body: dict) -> dict:
        data = json.dumps(body).encode()
        req = urllib.request.Request(
            f"{self.base_url}{path}",
            data=data,
            headers=self.headers,
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                return json.loads(resp.read())
        except urllib.error.HTTPError as e:
            error_body = e.read().decode()
            try:
                err = json.loads(error_body)
                raise RuntimeError(f"API error ({e.code}): {err.get('error', error_body)}")
            except json.JSONDecodeError:
                raise RuntimeError(f"API error ({e.code}): {error_body}")

    def run(
        self,
        code: str,
        language: str = "python",
        timeout: int = 30,
    ) -> Result:
        """Execute code in an isolated sandbox."""
        resp = self._request("/v1/exec", {
            "code": code,
            "language": language,
            "timeout_seconds": timeout,
        })
        return Result(**{k: v for k, v in resp.items() if k in Result.__dataclass_fields__})

    def run_batch(
        self,
        codes: list[str],
        language: str = "python",
        timeout: int = 30,
    ) -> list[Result]:
        """Execute multiple code snippets in parallel sandboxes."""
        resp = self._request("/v1/exec/batch", {
            "executions": [
                {"code": c, "language": language, "timeout_seconds": timeout}
                for c in codes
            ],
        })
        return [
            Result(**{k: v for k, v in r.items() if k in Result.__dataclass_fields__})
            for r in resp["results"]
        ]
