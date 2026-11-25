#!/usr/bin/env python3
"""Tests for the dev perception map's text presentation.

The test set was authored in a context that had not seen the implementation.
Run: python3 -m unittest discover -s scripts -p 'test_*.py'

SHARED-ORACLE is implemented in ``oracle_violations`` below and is the single
definition used by both the Python tests and the by-hand checks in
``docs/agents/desktop-map-checks.md``.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOOL = ROOT / "scripts" / "show-desktop-map.py"
ROWS = ROOT / "scripts" / "fixtures" / "probe-rows.log"
NO_COMPOSITOR = ROOT / "scripts" / "fixtures" / "no-compositor.log"
PERCEPTION = ROOT / "scripts" / "agent-perception"
PROTOCOLS = PERCEPTION / "protocols"
PROBE_HARNESS = ROOT / "scripts" / "test_probe_filter_harness.c"

LINE = re.compile(
    r"^(\d{2}) (\S+) \[(-?\d+),(-?\d+),(-?\d+),(-?\d+)\] "
    r"(normal|active|minimized|maximized|fullscreen)$"
)
PATH_RE = re.compile(r"/(home|usr|tmp|etc|var|Users)/|~/")
URL_RE = re.compile(r"[A-Za-z][A-Za-z0-9+.-]*://")
CRED_RE = re.compile(r"(password|passwd|secret|token|api[_-]?key|credential)\b.{0,8}[=:]")
HEX_BLOB_RE = re.compile(r"[0-9a-fA-F]{32,}")
B64_BLOB_RE = re.compile(r"[A-Za-z0-9+/]{40,}={0,2}")
FORBIDDEN_FIELD_RE = re.compile(
    r"\b(title|identifier|ident|pixels|pixel|path|url)\b\s*[:=]", re.IGNORECASE)

# Headless stand-in for Tk: it records any window construction instead of
# mapping one, so a deadline row can observe "before any window is mapped".
TKINTER_STUB = '''\
import json
import os
from pathlib import Path

_IMPORT_MARKER = os.environ.get("KOMOREBI_TEST_TK_IMPORTED")
if _IMPORT_MARKER:
    Path(_IMPORT_MARKER).write_text("tk", encoding="utf-8")
_TEXT_MARKER = os.environ.get("KOMOREBI_TEST_TK_TEXTS")

def _record_text(value):
    if _TEXT_MARKER:
        with Path(_TEXT_MARKER).open("a", encoding="utf-8") as fh:
            fh.write(str(value) + "\\n")

class _NoOp:
    def __getattr__(self, name):
        return lambda *args, **kwargs: None

class _Root(_NoOp):
    def __init__(self):
        self._pending = []

    def after(self, delay, callback=None, *args):
        marker = os.environ.get("KOMOREBI_TEST_TK_AFTER")
        if marker:
            with Path(marker).open("a", encoding="utf-8") as fh:
                fh.write(json.dumps({"delay_type": type(delay).__name__,
                                     "delay": str(delay),
                                     "callback": callable(callback)}) + "\\n")
        self._pending.append((callback, args))
        return None

    def mainloop(self, *args, **kwargs):
        pending, self._pending = self._pending, []
        for callback, cb_args in pending:
            if callable(callback):
                callback(*cb_args)
        return None

def Tk(*args, **kwargs):
    marker = os.environ.get("KOMOREBI_TEST_TK_CONSTRUCTED")
    if marker:
        Path(marker).write_text("tk", encoding="utf-8")
    return _Root()

class _Canvas(_NoOp):
    def create_text(self, *args, **kwargs):
        _record_text(kwargs.get("text", ""))
        return None

def Canvas(*args, **kwargs):
    return _Canvas()
'''


def fixture_rows(path: Path):
    """Parse the recorded probe log; expected values come from the fixture."""
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.startswith("DUMP|"):
            continue
        parts = line.split("|")
        app, ident, x, y, w, h, active, minimized, maximized, fullscreen = parts[1:11]
        state = "normal"
        for bit, label in ((active, "active"), (minimized, "minimized"),
                           (maximized, "maximized"), (fullscreen, "fullscreen")):
            if int(bit):
                state = label
        rows.append(dict(app=app, ident=ident, x=int(x), y=int(y), w=int(w),
                         h=int(h), state=state))
    return rows


def oracle_violations(text: str, allowed: set[str], opaque_ids: list[str], artifacts=None):
    """SHARED-ORACLE: only spec-named fields; no title, pixel, path, URL,
    credential-like token or opaque identifier in output or stored data."""
    violations = []
    for token in re.findall(r"\S+", text):
        if token not in allowed:
            violations.append("unallowed token: " + token)
    for name, regex in (("path", PATH_RE), ("url", URL_RE), ("credential", CRED_RE),
                        ("pixel", HEX_BLOB_RE), ("pixel", B64_BLOB_RE)):
        if regex.search(text):
            violations.append("forbidden category: " + name)
    for opaque in opaque_ids:
        if opaque and opaque in text:
            violations.append("opaque identifier in output")
    if FORBIDDEN_FIELD_RE.search(text):
        violations.append("forbidden field name")
    for name, payload in (artifacts or {}).items():
        decoded = payload.decode("utf-8", errors="replace")
        if oracle_violations(decoded, allowed, opaque_ids):
            violations.append("artifact violates oracle: " + name)
    return violations


def run(*args, cwd=None, env=None):
    return subprocess.run([sys.executable, str(TOOL), *map(str, args)],
                          cwd=cwd, env=env, capture_output=True, text=True,
                          timeout=30)


class PerceptionMapTextTests(unittest.TestCase):
    """Given 22 rows from a read-only compositor query
    When the developer asks for plain text
    Then every row is identified by a stable number and shows only application
         identity, rectangle and state
    And retained: fixture order and probe diagnostics are not presentation fields.
    """

    def test_text_rows_are_numbered_and_minimal(self):
        rows = fixture_rows(ROWS)
        proc = run("--rows", ROWS, "--text")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        lines = proc.stdout.splitlines()
        self.assertEqual(len(lines), len(rows))
        for i, (line, row) in enumerate(zip(lines, rows), 1):
            match = LINE.match(line)
            self.assertIsNotNone(match, line)
            number, app, x, y, w, h, state = match.groups()
            self.assertEqual(number, f"{i:02d}")
            self.assertEqual(app, row["app"])
            self.assertEqual((int(x), int(y), int(w), int(h)),
                             (row["x"], row["y"], row["w"], row["h"]))
            self.assertEqual(state, row["state"])
        examples = {
            5: ("Gitk", [1724, 40, 422, 1316], "minimized"),
            19: ("com.system76.CosmicTerm", [8, 40, 1708, 1316], "active"),
            22: ("chrome-ompifgpmddkgmclendfeacglnodjjndh-Default",
                 [8, 40, 3424, 1316], "normal"),
        }
        for number, (app, rect, state) in examples.items():
            self.assertEqual(lines[number - 1],
                             f"{number:02d} {app} [{','.join(map(str, rect))}] {state}")

    def test_cli_rows_pin_text_oracle_and_deadline_validation(self):
        """Given untrusted --rows log rows and CLI literals: the recorded
             22-row fixture, deadline values (nan, inf, -inf, +inf, -nan, zero,
             negative, finite positive 12/0.5, very large finite positive
             1e308/1e305), the five bad-app fields, and leading
             credential/blob-like runs followed by otherwise-allowed
             characters (32 hex characters + .x, 48 alphanumeric characters,
             and an AKIA-style token + .evil)
        When the developer runs text mode or a positive-deadline headless Tk-stub
             map presentation for one row
        Then no forbidden value or credential/blob-like run appears in stdout,
             stderr, any written artifact or any mapped text; each of the three
             leading-run rows is rejected with a named reason, maps no window
             and presents no invented identity, while the five existing bad-app
             rows keep their reject-or-minimal-identity outcomes
        And retained: the fixture renders stable numbers with only app/rect/state
             including the long chrome identity; CONNECT_FAIL invents no rows;
             every deadline row still behaves; SHARED-ORACLE passes on all
             output; text mode writes no artifact and imports no Tk; the five
             existing bad-app rows still behave; four Python test methods remain.

        SHARED-ORACLE is the single definition used by the Python tests and the
        by-hand checks in docs/agents/desktop-map-checks.md.
        """
        rows = fixture_rows(ROWS)
        allowed = {"CONNECT_FAIL"}
        for i, row in enumerate(rows, 1):
            allowed.update((f"{i:02d}", row["app"], row["state"],
                            f"[{row['x']},{row['y']},{row['w']},{row['h']}]"))
        opaque = [row["ident"] for row in rows]
        with tempfile.TemporaryDirectory() as tmp:
            fixture_copy = Path(tmp) / "probe-rows.log"
            shutil.copyfile(ROWS, fixture_copy)
            before = {p.name: p.read_bytes() for p in Path(tmp).iterdir()}
            first = run("--rows", fixture_copy, "--text", cwd=tmp)
            after = {p.name: p.read_bytes() for p in Path(tmp).iterdir()}
            changed = {k: v for k, v in after.items() if before.get(k) != v}
            self.assertEqual(first.returncode, 0, first.stderr)
            self.assertEqual(oracle_violations(first.stdout, allowed, opaque, changed), [])
            second = run("--rows", fixture_copy, "--text", cwd=tmp)
            self.assertEqual(second.stdout, first.stdout)
            # A text run writes no artifact: asserted, not skipped.
            self.assertEqual(changed, {})

            # Deadline rows use the same CLI and the same no-artifact snapshot.
            with tempfile.TemporaryDirectory() as stub_tmp:
                stub = Path(stub_tmp)
                (stub / "tkinter.py").write_text(TKINTER_STUB, encoding="utf-8")
                marker = stub / "TK_CONSTRUCTED"
                after_marker = stub / "TK_AFTER"
                env = os.environ.copy()
                env["PYTHONPATH"] = str(stub) + os.pathsep + env.get("PYTHONPATH", "")
                env["KOMOREBI_TEST_TK_CONSTRUCTED"] = str(marker)
                env["KOMOREBI_TEST_TK_AFTER"] = str(after_marker)
                env.pop("DISPLAY", None)
                env.pop("WAYLAND_DISPLAY", None)
                # Measured with tclsh 8.6.14 on this host: `after` accepts
                # delays through 2**64-1 and rejects larger integers.
                tcl_after_max = 2**64 - 1
                deadline_rows = (
                    ("nan", False), ("inf", False), ("-inf", False),
                    ("+inf", False), ("-nan", False),
                    ("0", False), ("-1", False),
                    ("12", True), ("0.5", True),
                    ("1e308", True), ("1e305", True),
                )
                for value, accepted in deadline_rows:
                    with self.subTest(deadline=value):
                        marker.unlink(missing_ok=True)
                        after_marker.unlink(missing_ok=True)
                        before_deadline = {
                            str(p.relative_to(tmp)): p.read_bytes()
                            for p in Path(tmp).rglob("*") if p.is_file()}
                        proc = run("--rows", fixture_copy, "--deadline", value,
                                   cwd=tmp, env=env)
                        after_deadline = {
                            str(p.relative_to(tmp)): p.read_bytes()
                            for p in Path(tmp).rglob("*") if p.is_file()}
                        scheduled = (
                            [json.loads(line) for line in
                             after_marker.read_text(encoding="utf-8").splitlines()]
                            if after_marker.exists() else [])
                        if accepted:
                            self.assertEqual(proc.returncode, 0,
                                             proc.stdout + proc.stderr)
                            self.assertTrue(
                                marker.exists(),
                                f"deadline {value} did not drive the map")
                            self.assertTrue(
                                scheduled,
                                f"deadline {value} scheduled no Tk callback")
                            for entry in scheduled:
                                self.assertEqual(entry["delay_type"], "int",
                                                 f"deadline {value} delay is "
                                                 f"{entry['delay_type']}, not int")
                                self.assertTrue(
                                    entry["callback"],
                                    f"deadline {value} scheduled no callable")
                                self.assertLessEqual(
                                    int(entry["delay"]), tcl_after_max,
                                    f"deadline {value} delay "
                                    f"{entry['delay']} exceeds Tcl after range")
                        else:
                            self.assertNotEqual(proc.returncode, 0,
                                                proc.stdout + proc.stderr)
                            self.assertFalse(
                                marker.exists(),
                                f"deadline {value} reached window construction")
                            error = (proc.stdout + proc.stderr).lower()
                            self.assertIn("deadline", error, proc.stderr)
                            self.assertRegex(error, r"\b(?:invalid|finite|positive)\b",
                                             proc.stderr)
                        self.assertEqual(after_deadline, before_deadline)

                # Bad app-identity rows: Given a --rows log whose app field is a
                # path, URL, credential-like token, or a credential/blob-like
                # leading run followed by otherwise-allowed characters; When
                # text and positive-deadline headless-map presentations run; Then
                # no forbidden value reaches stdout, stderr, an artifact or canvas
                # text; the three leading-run rows are rejected with a named
                # reason and map no window, and the five original rows keep their
                # reject-or-minimal-identity outcomes.
                # Retained: fixture, CONNECT_FAIL, deadlines, SHARED-ORACLE,
                # no-artifact/no-Tk text mode and four test methods remain pinned.
                imported = stub / "TK_IMPORTED"
                texts = stub / "TK_TEXTS"
                env["KOMOREBI_TEST_TK_IMPORTED"] = str(imported)
                env["KOMOREBI_TEST_TK_TEXTS"] = str(texts)

                def snapshot_dir():
                    return {str(p.relative_to(tmp)): p.read_bytes()
                            for p in Path(tmp).rglob("*") if p.is_file()}

                def caps(text):
                    return set(re.findall(r"\b[A-Z][A-Z0-9_]*\b", text))

                def reset_markers():
                    for path in (imported, texts, marker, after_marker):
                        path.unlink(missing_ok=True)

                def check_bad(name, line, forbidden, prefix, rect, state,
                              opaque=(), reason=None):
                    fixture = Path(tmp) / f"bad-{name}.log"
                    fixture.write_text(line + "\n", encoding="utf-8")
                    rect_token = "[" + ",".join(map(str, rect)) + "]"

                    # Text mode: stdout AND stderr go to SHARED-ORACLE.
                    reset_markers()
                    before = snapshot_dir()
                    proc = run("--rows", fixture, "--text", cwd=tmp, env=env)
                    changed = {k: v for k, v in snapshot_dir().items()
                               if before.get(k) != v}
                    out = proc.stdout + proc.stderr
                    for bad in forbidden:
                        self.assertNotIn(
                            bad, out, f"{name}: forbidden text value {bad!r}")
                    self.assertEqual(changed, {}, name)
                    self.assertFalse(imported.exists(), name)
                    self.assertFalse(marker.exists(), name)
                    rows = [text_line for text_line in out.splitlines()
                            if LINE.match(text_line)]
                    reasons = caps(out)
                    self.assertNotEqual(bool(rows), bool(reasons), name)
                    allowed = {"01", rect_token, state} | reasons
                    if prefix is None:
                        self.assertEqual(rows, [], name)
                        self.assertTrue(reasons, f"{name}: named reason")
                    else:
                        self.assertEqual(len(rows), 1, name)
                        g = LINE.match(rows[0]).groups()
                        self.assertEqual(
                            (g[1], tuple(map(int, g[2:6])), g[6]),
                            (prefix, rect, state), name)
                        allowed.add(prefix)
                    if reason:
                        self.assertEqual(
                            [line.strip() for line in out.splitlines()
                             if line.strip()], [reason],
                            f"{name}: text presentation is exactly {reason}")
                    self.assertEqual(
                        oracle_violations(out, allowed, list(opaque), changed), [], name)

                    # Mapped mode under the headless stub: no real window opens.
                    reset_markers()
                    before_map = snapshot_dir()
                    proc = run("--rows", fixture, "--deadline", "12",
                               cwd=tmp, env=env)
                    changed_map = {k: v for k, v in snapshot_dir().items()
                                   if before_map.get(k) != v}
                    self.assertEqual(changed_map, {}, name)
                    mapped_out = proc.stdout + proc.stderr
                    presented = (texts.read_text(encoding="utf-8").splitlines()
                                 if texts.exists() else [])
                    for bad in forbidden:
                        self.assertNotIn(
                            bad, mapped_out, f"{name}: forbidden output value {bad!r}")
                        self.assertNotIn(
                            bad, "\n".join(presented),
                            f"{name}: forbidden canvas value {bad!r}")
                    mapped_rows = [text_line for text_line in presented
                                   if LINE.match(text_line)]
                    mapped_reasons = caps("\n".join([*presented,
                                                     *mapped_out.splitlines()]))
                    self.assertNotEqual(bool(mapped_rows), bool(mapped_reasons), name)
                    allowed_map = {"01", rect_token, state} | mapped_reasons
                    if prefix is None:
                        self.assertFalse(marker.exists(), name)
                        self.assertEqual(mapped_rows, [], name)
                        self.assertTrue(mapped_reasons, f"{name}: named reason")
                    else:
                        self.assertTrue(marker.exists(), name)
                        self.assertTrue(imported.exists(), name)
                        self.assertEqual(len(mapped_rows), 1, name)
                        g = LINE.match(mapped_rows[0]).groups()
                        self.assertEqual(
                            (g[1], tuple(map(int, g[2:6])), g[6]),
                            (prefix, rect, state), name)
                        allowed_map.add(prefix)
                    if reason:
                        self.assertEqual(
                            [line.strip() for line in mapped_out.splitlines()
                             if line.strip()], [reason],
                            f"{name}: mapped presentation is exactly {reason}")
                        self.assertEqual(presented, [],
                                         f"{name}: rejected row presents canvas text")
                    self.assertEqual(
                        oracle_violations(
                            mapped_out + "\n" + "\n".join(presented),
                            allowed_map, list(opaque), changed_map), [], name)

                reset_markers()
                probe_text = run("--rows", fixture_copy, "--text", cwd=tmp, env=env)
                self.assertEqual(probe_text.returncode, 0, probe_text.stderr)
                self.assertFalse(imported.exists(), "text mode imports no Tk")

                bad_app_rows = (
                    # path: leading "/" leaves no probe-filter identity
                    ("path",
                     "ROW|/home/alexv/.config/com.example.App/secrets.ini|"
                     "8|40|3424|1316|0|0|0|0",
                     ("/home/", "com.example.App/secrets.ini", "secrets.ini"),
                     None, (8, 40, 3424, 1316), "normal", ()),
                    # URL: maximal allowed leading run is "https"
                    ("url",
                     "ROW|https://user:token9f3a@portal.invalid/app|"
                     "8|40|1708|1316|1|0|0|0",
                     ("https://", "user:token9f3a", "portal.invalid",
                      "@portal.invalid/app"),
                     "https", (8, 40, 1708, 1316), "active", ()),
                    # credential field with separator: allowed leading run "api_key"
                    ("credential-field",
                     "ROW|api_key=AKIAEXAMPLE1234567890|1724|40|422|1316|0|1|0|0",
                     ("api_key=", "AKIAEXAMPLE1234567890"),
                     "api_key", (1724, 40, 422, 1316), "minimized", ()),
                    # bare credential token: filter passes it, spec forbids it
                    ("credential-token",
                     "ROW|AKIAEXAMPLE1234567890|864|40|852|654|0|1|0|0",
                     ("AKIAEXAMPLE1234567890",),
                     None, (864, 40, 852, 654), "minimized", ()),
                    # legacy form: punctuation stops at "com.example.App"
                    ("non-minimal-dump",
                     "DUMP|com.example.App(1)|"
                     "9f3ad41c7b2e4a5180cd6f27ba39e412|8|702|418|654|0|1|0|0",
                     ("com.example.App(1)", "(1)",
                      "9f3ad41c7b2e4a5180cd6f27ba39e412"),
                     "com.example.App", (8, 702, 418, 654), "minimized",
                     ("9f3ad41c7b2e4a5180cd6f27ba39e412",)),
                    # 32-character hex-like run followed by allowed ".x": the
                    # whole field is probe-shaped, but the run is forbidden.
                    ("hex32-plus-suffix",
                     "ROW|9f3ad41c7b2e4a5180cd6f27ba39e412.x|"
                     "8|702|418|654|0|1|0|0",
                     ("9f3ad41c7b2e4a5180cd6f27ba39e412",
                      "9f3ad41c7b2e4a5180cd6f27ba39e412.x"),
                     None, (8, 702, 418, 654), "minimized", ()),
                    # 48-character alphanumeric run: B64_BLOB_RE forbids 40+.
                    ("alnum48-leading-run",
                     "ROW|Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56Qr78St90Uv12Wx34|"
                     "864|40|852|654|0|1|0|0",
                     ("Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56Qr78St90Uv12Wx34",),
                     None, (864, 40, 852, 654), "minimized", ()),
                    # AKIA-style token followed by allowed ".evil": the token
                    # alone is already forbidden by the existing bad-app row.
                    ("akia-plus-suffix",
                     "ROW|AKIAEXAMPLE1234567890.evil|"
                     "8|40|1708|1316|1|0|0|0",
                     ("AKIAEXAMPLE1234567890", "AKIAEXAMPLE1234567890.evil",
                      ".evil"),
                     None, (8, 40, 1708, 1316), "active", ()),
                    # Given an untrusted --rows log whose ROW application-identity
                    # field is the hyphen-separated UUID from the spec; When the
                    # text or positive-deadline headless presentation runs; Then
                    # no UUID, segment or replacement identity is emitted, the row
                    # is rejected with INVALID_ROW and no window maps. And retained:
                    # the long chrome identity and all 22 fixture rows still render.
                    ("uuid-row",
                     "ROW|123e4567-e89b-12d3-a456-426614174000|"
                     "8|40|3424|1316|1|0|0|0",
                     ("123e4567-e89b-12d3-a456-426614174000", "123e4567",
                      "123e4567-e89b", "426614174000"),
                     None, (8, 40, 3424, 1316), "active",
                     ("123e4567-e89b-12d3-a456-426614174000",), "INVALID_ROW"),
                    # Given the same UUID in the legacy DUMP application-identity
                    # column plus a real fixture opaque identifier in column two;
                    # When either presentation runs; Then neither value appears in
                    # output, artifact or canvas text, INVALID_ROW is named and no
                    # window maps. And retained: DUMP rows are still accepted, the
                    # opaque column is still discarded and CONNECT_FAIL still invents
                    # no rows.
                    ("uuid-dump-row",
                     "DUMP|123e4567-e89b-12d3-a456-426614174000|"
                     "QhC412DNHEcwUW6KblOIHbTqciDDDJt4|8|40|3424|1316|0|1|0|0",
                     ("123e4567-e89b-12d3-a456-426614174000", "123e4567",
                      "426614174000", "QhC412DNHEcwUW6KblOIHbTqciDDDJt4"),
                     None, (8, 40, 3424, 1316), "minimized",
                     ("123e4567-e89b-12d3-a456-426614174000",
                      "QhC412DNHEcwUW6KblOIHbTqciDDDJt4"), "INVALID_ROW"),
                )
                for name, line, forbidden, prefix, rect, state, opaque, *rest in bad_app_rows:
                    with self.subTest(row=name):
                        check_bad(name, line, forbidden, prefix, rect, state,
                                  opaque, reason=rest[0] if rest else None)

    def test_no_compositor_reports_named_reason_without_rows(self):
        """Given no compositor publishes the required window information
        When the presentation is requested
        Then the absence is reported with the named reason
        And no row is invented and no window is mapped.
        """
        proc = run("--rows", NO_COMPOSITOR, "--text")
        self.assertIn("CONNECT_FAIL", proc.stdout)
        self.assertEqual([line for line in proc.stdout.splitlines() if LINE.match(line)], [])
        for row in fixture_rows(ROWS):
            self.assertNotIn(row["app"], proc.stdout)

    def test_probe_filter_rejects_blob_like_application_identities(self):
        """Given a compositor publishes an application identity shaped as a
             hyphen-separated UUID, 32 contiguous hex characters, 40 contiguous
             alphanumeric characters or an AKIA-style token
        When the read-only probe consumes a foreign-toplevel app_id event or a
             COSMIC toplevel app_id event and emits its row data (compiled harness,
             no compositor, no surface, no input)
        Then the blob-like value is absent from stdout, INVALID_ROW is named and
             no ROW| line carrying the value (or any invented row) is emitted
        And retained: org.keepassxc.KeePassXC and the long chrome identity still
             pass through unchanged; CONNECT_FAIL and NO_COSMIC_TOPLEVEL_INFO are
             unchanged; build.sh builds with installed tooling and leaves only
             the probe binary; no title, identifier, path, URL, credential-like
             token or pixel data is emitted.
        """
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            generated = root / "generated"
            generated.mkdir()

            def tool(*argv):
                return subprocess.run([*map(str, argv)], capture_output=True,
                                      text=True, timeout=60)

            for xml in sorted(PROTOCOLS.glob("*.xml")):
                for kind, suffix in (("client-header", ".h"),
                                     ("private-code", ".c")):
                    made = tool("wayland-scanner", kind, xml,
                                generated / (xml.stem + suffix))
                    self.assertEqual(made.returncode, 0, made.stderr)

            cflags = tool("pkg-config", "--cflags", "wayland-client")
            libs = tool("pkg-config", "--libs", "wayland-client")
            self.assertEqual(cflags.returncode, 0, cflags.stderr)
            self.assertEqual(libs.returncode, 0, libs.stderr)
            harness = root / "probe-filter-harness"
            compiled = tool(
                "cc", "-std=c11", "-O0", "-Wall", "-Wextra",
                f"-I{generated}", f"-I{PERCEPTION}", *cflags.stdout.split(),
                PROBE_HARNESS, *sorted(generated.glob("*.c")),
                "-o", harness, *libs.stdout.split())
            self.assertEqual(compiled.returncode, 0, compiled.stderr)

            def probe(*args):
                before = {p.name: p.read_bytes() for p in root.iterdir()
                          if p.is_file()}
                proc = subprocess.run([str(harness), *map(str, args)], cwd=root,
                                      capture_output=True, text=True, timeout=30)
                after = {p.name: p.read_bytes() for p in root.iterdir()
                         if p.is_file()}
                self.assertEqual(after, before, f"{args}: probe wrote an artifact")
                return proc

            # Both compositor app_id events are observed independently: one run
            # delivers the value on the foreign toplevel, one on the COSMIC
            # toplevel. The falsifier that filters only the first must fail.
            listeners = ("--foreign", "--cosmic")
            blob_rows = (
                ("uuid", "123e4567-e89b-12d3-a456-426614174000"),
                ("hex32", "9f3ad41c7b2e4a5180cd6f27ba39e412"),
                ("akia", "AKIAEXAMPLE1234567890"),
                ("alnum40", "Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56Qr78St90"),
            )
            for name, value in blob_rows:
                for listener in listeners:
                    with self.subTest(row=name, event=listener):
                        proc = probe(listener, value)
                        self.assertEqual(proc.returncode, 0, proc.stderr)
                        emitted = proc.stdout + proc.stderr
                        self.assertNotIn(value, emitted,
                                         f"{name}/{listener}: blob value leaked")
                        self.assertIn("INVALID_ROW", proc.stdout,
                                      f"{name}/{listener}: named reason missing")
                        row_lines = [line for line in proc.stdout.splitlines()
                                     if line.startswith("ROW|")]
                        self.assertEqual(row_lines, [],
                                         f"{name}/{listener}: rejected row invented a ROW line")
                        allowed = {"READY", "ft=1", "cosmic=1", "tops=1",
                                   "INVALID_ROW"}
                        self.assertEqual(
                            oracle_violations(emitted, allowed, [value]),
                            [], f"{name}/{listener}")

            normal_rows = (
                ("keepassxc", "org.keepassxc.KeePassXC"),
                ("chrome", "chrome-ompifgpmddkgmclendfeacglnodjjndh-Default"),
            )
            for name, value in normal_rows:
                for listener in listeners:
                    with self.subTest(row=name, event=listener):
                        proc = probe(listener, value)
                        self.assertEqual(proc.returncode, 0, proc.stderr)
                        emitted = proc.stdout + proc.stderr
                        expected = f"ROW|{value}|8|40|3424|1316|0|0|0|0"
                        self.assertEqual(
                            [line for line in proc.stdout.splitlines()
                             if line.startswith("ROW|")], [expected],
                            f"{name}/{listener}")
                        self.assertNotIn("INVALID_ROW", proc.stdout,
                                         f"{name}/{listener}")
                        allowed = {"READY", "ft=1", "cosmic=1", "tops=1", expected}
                        self.assertEqual(
                            oracle_violations(emitted, allowed, []), [],
                            f"{name}/{listener}")

            for argument, reason, expected_rc in (
                    ("--no-compositor", "CONNECT_FAIL", 2),
                    ("--no-cosmic", "NO_COSMIC_TOPLEVEL_INFO", 0)):
                with self.subTest(reason=reason):
                    proc = probe(argument)
                    self.assertEqual(proc.returncode, expected_rc, proc.stderr)
                    self.assertEqual(proc.stdout.strip(), reason)
                    self.assertEqual(proc.stderr, "")
                    self.assertEqual(
                        oracle_violations(proc.stdout, {reason}, []), [], reason)

            build_dir = root / "build"
            build_dir.mkdir()
            build = tool("bash", PERCEPTION / "build.sh", build_dir / "probe")
            self.assertEqual(build.returncode, 0, build.stderr)
            self.assertIn("PROBE_BUILT", build.stdout)
            self.assertEqual(sorted(p.name for p in build_dir.iterdir()),
                             ["probe"])
            env = os.environ.copy()
            env["WAYLAND_DISPLAY"] = "perception-missing"
            disconnected = subprocess.run([str(build_dir / "probe")],
                                          capture_output=True, text=True,
                                          timeout=30, env=env)
            self.assertEqual(disconnected.returncode, 2, disconnected.stderr)
            self.assertEqual(disconnected.stdout.strip(), "CONNECT_FAIL")


if __name__ == "__main__":
    unittest.main()
