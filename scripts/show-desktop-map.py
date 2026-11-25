#!/usr/bin/env python3
"""Dev-only perception map: show read-only desktop window rows as signal.

The rows are a mirror of what a read-only compositor query observed. This tool
is not an observation source for the application or the agent: it never changes
`state.json`, never promises a capability, and never maps a window unless a
positive deadline is supplied.

Text mode needs no GUI toolkit and writes no artifact:

    python3 scripts/show-desktop-map.py --rows scripts/fixtures/probe-rows.log --text
    python3 scripts/show-desktop-map.py --text

Map mode opens one black/white window and closes it at the deadline:

    python3 scripts/show-desktop-map.py --rows scripts/fixtures/probe-rows.log --deadline 12
"""
from __future__ import annotations

import argparse
import math
import os
import re
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_PROBE = ROOT / "scripts" / "agent-perception" / "probe"
# Tk's `after` takes an integer delay. 2**31-1 ms is portable across the
# 32-bit and 64-bit Tcl builds this dev tool supports; larger deadlines are
# re-armed in bounded slices instead of overflowing the conversion.
MAX_AFTER_MS = 2 ** 31 - 1

ROW_RE = re.compile(
    r"^ROW\|([^|]+)\|(-?\d+)\|(-?\d+)\|(-?\d+)\|(-?\d+)\|"
    r"([01])\|([01])\|([01])\|([01])$"
)
REASONS = ("CONNECT_FAIL", "NO_COSMIC_TOPLEVEL_INFO", "INVALID_ROW")
# Same minimal identity shape as the probe's app_id filter. A row log is an
# untrusted input too, so presentation passes through this boundary as well.
APP_PREFIX_RE = re.compile(r"[A-Za-z0-9._-]+")
# The forbidden token shapes from SHARED-ORACLE. Test the identity that would
# actually be presented, not the whole raw field: a blob followed by an
# allowed suffix would otherwise pass as an observation.
IDENTITY_BLOB_RE = re.compile(
    r"(?:[0-9a-fA-F]{32,}|[A-Za-z0-9+/]{40,}={0,2}|(?:AKIA|ASIA)[A-Z0-9]{12,}"
    r"|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})"
)
STATES = (("activated", "active"), ("minimized", "minimized"),
          ("maximized", "maximized"), ("fullscreen", "fullscreen"))


@dataclass(frozen=True)
class Row:
    app: str
    x: int
    y: int
    w: int
    h: int
    state: str

    def text(self, number: int) -> str:
        return f"{number:02d} {self.app} [{self.x},{self.y},{self.w},{self.h}] {self.state}"


def _state(bits: tuple[str, str, str, str]) -> str:
    result = "normal"
    for value, label in zip(bits, (s for _, s in STATES)):
        if value == "1":
            result = label
    return result


def _minimal_app(raw: str) -> str | None:
    """Return only the probe-shaped identity, or reject a non-minimal field."""
    match = APP_PREFIX_RE.match(raw)
    if not match:
        return None
    app = match.group(0)
    if IDENTITY_BLOB_RE.search(app):
        return None
    return app


def parse_lines(lines) -> tuple[str | None, list[Row]]:
    """Read only the spec-named fields from probe or fixture lines."""
    reason = None
    rows = []
    for raw in lines:
        line = raw.strip()
        if line in REASONS:
            reason = line
            continue
        match = ROW_RE.match(line)
        if match:
            app = _minimal_app(match.group(1))
            if app is None:
                reason = reason or "INVALID_ROW"
                continue
            rows.append(Row(app, int(match.group(2)), int(match.group(3)),
                            int(match.group(4)), int(match.group(5)),
                            _state(match.group(6, 7, 8, 9))))
            continue
        if line.startswith("DUMP|"):
            # Legacy recorded fixtures: column 2 is an opaque identifier and
            # is deliberately discarded.
            parts = line.split("|")
            if len(parts) < 11:
                continue
            app = _minimal_app(parts[1])
            if app is None:
                reason = reason or "INVALID_ROW"
                continue
            rows.append(Row(app, int(parts[3]), int(parts[4]), int(parts[5]),
                            int(parts[6]), _state(tuple(parts[7:11]))))
    return reason, rows


def _read_rows(path: str | None) -> tuple[str | None, list[Row]]:
    if path:
        try:
            return parse_lines(Path(path).read_text(encoding="utf-8").splitlines())
        except (OSError, UnicodeError, ValueError):
            return "ROWS_UNAVAILABLE", []
    probe = Path(os.environ.get("KOMOREBI_PERCEPTION_PROBE", str(DEFAULT_PROBE)))
    if not probe.is_file():
        return "PROBE_NOT_BUILT", []
    try:
        proc = subprocess.run([str(probe)], capture_output=True, text=True,
                              timeout=10, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return "PROBE_UNAVAILABLE", []
    return parse_lines(proc.stdout.splitlines())


def emit_text(reason: str | None, rows: list[Row]) -> int:
    if reason:
        print(reason)
        return 2
    if not rows:
        print("NO_ROWS")
        return 2
    for number, row in enumerate(rows, 1):
        print(row.text(number))
    return 0


def _positive(value: str) -> float:
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError(
            "deadline must be finite and positive seconds")
    return number


def _attach_deadline_value(argv: list[str]) -> list[str]:
    """Keep signed deadline literals as values, not parser options."""
    result = []
    i = 0
    while i < len(argv):
        if (argv[i] == "--deadline" and i + 1 < len(argv)
                and argv[i + 1].startswith("-") and not argv[i + 1].startswith("--")):
            result.append(f"--deadline={argv[i + 1]}")
            i += 2
        else:
            result.append(argv[i])
            i += 1
    return result


def emit_map(reason: str | None, rows: list[Row], deadline: float) -> int:
    if reason:
        print(reason)
        return 2
    if not rows:
        print("NO_ROWS")
        return 2
    try:
        import tkinter as tk
    except ImportError:
        print("TK_UNAVAILABLE")
        return 2

    width, height = 1200, 800
    map_h, pad = 470, 18
    root = tk.Tk()
    root.title("Perception map")
    root.geometry(f"{width}x{height}")
    canvas = tk.Canvas(root, width=width, height=height, bg="black",
                       highlightthickness=0)
    canvas.pack(fill="both", expand=True)

    min_x = min(row.x for row in rows)
    min_y = min(row.y for row in rows)
    max_x = max(row.x + row.w for row in rows)
    max_y = max(row.y + row.h for row in rows)
    span_x = max(1, max_x - min_x)
    span_y = max(1, max_y - min_y)
    scale = min((width - 2 * pad) / span_x, (map_h - 2 * pad) / span_y)
    off_x = (width - span_x * scale) / 2
    off_y = (map_h - span_y * scale) / 2

    for number, row in enumerate(rows, 1):
        x0 = off_x + (row.x - min_x) * scale
        y0 = off_y + (row.y - min_y) * scale
        x1 = x0 + max(2, row.w * scale)
        y1 = y0 + max(2, row.h * scale)
        canvas.create_rectangle(x0, y0, x1, y1, outline="white", width=2)
        canvas.create_text(x0 + 3, y0 + 3, text=f"{number:02d}", fill="white",
                           anchor="nw", font=("monospace", 10))

    canvas.create_line(pad, map_h, width - pad, map_h, fill="white")
    for number, row in enumerate(rows, 1):
        canvas.create_text(pad, map_h + 10 + (number - 1) * 22,
                           text=row.text(number), fill="white", anchor="nw",
                           font=("monospace", 13))

    close_at = time.monotonic() + deadline

    def schedule_close() -> None:
        remaining_ms = (close_at - time.monotonic()) * 1000
        if remaining_ms <= 0:
            root.destroy()
            return
        delay_ms = (MAX_AFTER_MS if remaining_ms >= MAX_AFTER_MS
                    else max(1, int(remaining_ms)))
        root.after(delay_ms, schedule_close)

    schedule_close()
    root.mainloop()
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--text", action="store_true", help="print rows, open no window")
    mode.add_argument("--deadline", type=_positive, metavar="SECONDS",
                      help="open one map window and close it after SECONDS")
    parser.add_argument("--rows", metavar="LOG",
                        help="read a recorded probe log instead of querying live")
    args = parser.parse_args(_attach_deadline_value(sys.argv[1:]))
    reason, rows = _read_rows(args.rows)
    if args.text:
        return emit_text(reason, rows)
    return emit_map(reason, rows, args.deadline)


if __name__ == "__main__":
    sys.exit(main())
