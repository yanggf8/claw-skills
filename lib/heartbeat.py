"""Wall-clock heartbeat for long-running skill phases.

The nullclaw agent CLI does not stream tokens - stdout/stderr only flush on
completion - so byte-based stall detection gives false positives. This module
instead runs a background thread that writes a status file every N seconds,
giving external watchdogs a "last seen alive" timestamp.

Usage:

    from lib.heartbeat import HeartbeatWriter, run_with_heartbeat

    status_path = Path.home() / ".nullclaw/skills/my-skill/status.json"
    result = run_with_heartbeat(
        cmd=["nullclaw", "agent", "-m", prompt],
        status_path=status_path,
        run_id="2026-04-15T07:00:00",
        phase="writer",
        hard_timeout_secs=300,
        heartbeat_interval_secs=5,
    )
    # result.stdout / result.returncode / result.timed_out / result.error
"""
import json
import os
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional, Sequence


@dataclass
class RunResult:
    stdout: str
    stderr: str
    returncode: int
    timed_out: bool
    error: Optional[str]
    elapsed_secs: float


class HeartbeatWriter:
    """Background thread that periodically writes a status.json file.

    Lifecycle:
        hb = HeartbeatWriter(path, run_id, phase)
        hb.start()
        ... long operation ...
        hb.stop(final_phase="writer_done")
    """

    def __init__(
        self,
        status_path: Path,
        run_id: str,
        phase: str,
        interval_secs: float = 5.0,
        extra: Optional[dict] = None,
        stdout_interval_secs: Optional[float] = None,
    ):
        self.status_path = Path(status_path)
        self.run_id = run_id
        self.phase = phase
        self.interval_secs = interval_secs
        self.extra = extra or {}
        self.stdout_interval_secs = stdout_interval_secs
        self._started_at = 0.0
        self._last_stdout_at = 0.0
        self._stop = threading.Event()
        self._thread: Optional[threading.Thread] = None

    def _write(self, **overrides) -> None:
        payload = {
            "run_id": self.run_id,
            "phase": self.phase,
            "started_at": _iso(self._started_at),
            "last_heartbeat_at": _iso(time.time()),
            "stage_elapsed_sec": round(time.time() - self._started_at, 1),
            "pid": os.getpid(),
            **self.extra,
            **overrides,
        }
        tmp = self.status_path.with_suffix(".json.tmp")
        try:
            self.status_path.parent.mkdir(parents=True, exist_ok=True)
            tmp.write_text(json.dumps(payload, ensure_ascii=False))
            tmp.replace(self.status_path)
        except OSError as e:
            print(f"[heartbeat] write failed: {e}", file=sys.stderr)

    def _run(self) -> None:
        while not self._stop.is_set():
            self._write()
            self._maybe_stdout()
            self._stop.wait(self.interval_secs)

    def _maybe_stdout(self) -> None:
        if not self.stdout_interval_secs:
            return
        now = time.time()
        if now - self._last_stdout_at < self.stdout_interval_secs:
            return
        self._last_stdout_at = now
        elapsed = int(now - self._started_at)
        print(
            f"[{self.phase}] still running ({elapsed}s elapsed)",
            file=sys.stderr,
            flush=True,
        )

    def start(self) -> None:
        self._started_at = time.time()
        self._last_stdout_at = self._started_at
        self._write(phase_state="started")
        if self.stdout_interval_secs:
            print(
                f"[{self.phase}] started",
                file=sys.stderr,
                flush=True,
            )
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def stop(self, final_phase_state: str = "done", **final_fields) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=2.0)
        self._write(phase_state=final_phase_state, **final_fields)
        if self.stdout_interval_secs:
            elapsed = int(time.time() - self._started_at)
            print(
                f"[{self.phase}] {final_phase_state} ({elapsed}s)",
                file=sys.stderr,
                flush=True,
            )

    def read_status(self) -> dict:
        try:
            return json.loads(self.status_path.read_text())
        except (OSError, json.JSONDecodeError):
            return {}


def run_with_heartbeat(
    cmd: Sequence[str],
    status_path: Path,
    run_id: str,
    phase: str,
    hard_timeout_secs: int,
    heartbeat_interval_secs: float = 5.0,
    extra: Optional[dict] = None,
    stdout_interval_secs: Optional[float] = None,
) -> RunResult:
    """Run a subprocess with a wall-clock heartbeat thread and hard timeout.

    Failure taxonomy (distinct RunResult states):
      * returncode == 0            -> success
      * timed_out == True          -> hard timeout triggered, process killed
      * returncode != 0            -> subprocess exited non-zero
      * error != None              -> spawn or other exception
    """
    hb = HeartbeatWriter(
        status_path=status_path,
        run_id=run_id,
        phase=phase,
        interval_secs=heartbeat_interval_secs,
        extra=extra,
        stdout_interval_secs=stdout_interval_secs,
    )
    hb.start()
    started = time.time()
    try:
        proc = subprocess.Popen(
            list(cmd),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
        )
    except (OSError, ValueError) as e:
        hb.stop(final_phase_state="spawn_error", error=str(e))
        return RunResult("", "", -1, False, f"spawn failed: {e}", 0.0)

    try:
        stdout, stderr = proc.communicate(timeout=hard_timeout_secs)
    except subprocess.TimeoutExpired:
        proc.kill()
        try:
            stdout, stderr = proc.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            stdout, stderr = "", ""
        elapsed = time.time() - started
        hb.stop(
            final_phase_state="hard_timeout",
            timeout_secs=hard_timeout_secs,
            elapsed=round(elapsed, 1),
        )
        return RunResult(
            stdout or "",
            stderr or "",
            -1,
            True,
            f"hard timeout after {hard_timeout_secs}s",
            elapsed,
        )

    elapsed = time.time() - started
    hb.stop(
        final_phase_state="exited",
        returncode=proc.returncode,
        elapsed=round(elapsed, 1),
    )
    if proc.returncode != 0:
        return RunResult(
            stdout,
            stderr,
            proc.returncode,
            False,
            f"exit code {proc.returncode}",
            elapsed,
        )
    return RunResult(stdout, stderr, 0, False, None, elapsed)


def _iso(ts: float) -> str:
    if ts <= 0:
        return ""
    return datetime.fromtimestamp(ts, tz=timezone.utc).isoformat()
