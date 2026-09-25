#!/usr/bin/env python3
#
# Copyright (c) Memfault, Inc.
# See License.txt for details
"""Local hawkBit DDI mock and range server for zchunk delta OTA testing.

Exercises the SWUpdate delta handler with no Memfault backend. The device polls
this script through suricatta, is offered a deployment carrying the .swu and the
.zck, and fetches chunks with HTTP range requests. Needs a SWUpdate built with
CONFIG_DELTA=y and CONFIG_ZSTD=y and a base-image built with the ext4.zck and
ext4.zck.zckheader image types; see DEVELOPMENT.md.

    # what the device polls; leave running
    ./delta_ota_local_server.py ddi tmp/deploy/images/qemuarm64

    # on the device
    sed -i 's|"base_url": ".*"|"base_url": "http://10.0.2.2:8080"|' /etc/memfaultd.conf
    systemctl restart memfaultd swupdate

`pack` rewrites the .swu to a literal URL instead of `url = "dynamic"`, so it
can be installed by hand with no suricatta; `serve` then serves the artifacts.

    ./delta_ota_local_server.py pack  tmp/deploy/images/qemuarm64
    ./delta_ota_local_server.py serve tmp/deploy/images/qemuarm64
    # device: swupdate -H qemuarm64:1.0 -e stable,copy2 -i <name>-local.swu

10.0.2.2 is the host as seen from QEMU user-mode networking; override with
--host. The per-file byte totals printed on exit are the delta measurement;
--summary-json writes them, with the response status counts and the bounds of
every range served, as JSON.
"""

import argparse
import hashlib
import itertools
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, ClassVar, Protocol, TypedDict, cast

_DEFAULT_HOST = "10.0.2.2"
_DEFAULT_PORT = 8080
_DEFAULT_TENANT = "default"
_DEFAULT_DEVICE_ID = "qemu-tester"
# memfaultd appends this to base_url when it writes the suricatta section.
_HAWKBIT_PATH = "/api/v0/hawkbit"
_DOWNLOAD_PREFIX = "/download/"
_DELTA_SWU_GLOB = "swupdate-delta-image-*.swu"
_ACTION_ID = 1

JsonDict = dict[str, Any]  # pyright: ignore[reportExplicitAny]


class ArtifactSummary(TypedDict):
    size: int | None
    requests: int
    bytes: int
    statuses: dict[str, int]
    request_ranges: list[tuple[int, int]]


class Summary(TypedDict):
    version: str
    artifacts: dict[str, ArtifactSummary]


def _log(message: str) -> None:
    print(message, flush=True)


def _one_of(candidates: list[Path]) -> list[Path]:
    """Resolve deploy-dir glob matches, preferring the IMAGE_LINK_NAME symlink.

    Every artifact is deployed twice, once as IMAGE_NAME with a timestamp and
    once as a symlink to the newest of those.
    """
    links = [p for p in candidates if p.is_symlink()]
    return sorted({p.resolve() for p in (links or candidates)})


def _find_swu(deploy: Path, pattern: str) -> Path:
    matches = _one_of([p for p in deploy.glob(pattern) if "-local" not in p.name])
    if not matches:
        sys.exit(f"no {pattern} in {deploy}; run bitbake swupdate-delta-image")
    if len(matches) > 1:
        sys.exit(f"multiple {pattern} in {deploy}, pass --swu: {[p.name for p in matches]}")
    return matches[0]


def _unpack_swu(swu: Path, work: Path) -> list[str]:
    with swu.open("rb") as f:
        members = subprocess.run(
            ["cpio", "-t", "--quiet"],
            stdin=f,
            cwd=work,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.split()
    with swu.open("rb") as f:
        subprocess.run(["cpio", "-idmu", "--quiet"], stdin=f, cwd=work, check=True)
    return members


def _prop(desc: str, name: str) -> str | None:
    match = re.search(rf'\b{name}\s*=\s*"([^"]*)"', desc)
    return match.group(1) if match else None


def _hashes(path: Path) -> dict[str, str]:
    digests = {"md5": hashlib.md5(), "sha1": hashlib.sha1(), "sha256": hashlib.sha256()}
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            for digest in digests.values():
                digest.update(block)
    return {name: digest.hexdigest() for name, digest in digests.items()}


def _resolve_artifact(deploy: Path, name: str) -> Path:
    direct = deploy / name
    if direct.is_file():
        return direct
    head, _, tail = name.partition(".")
    suffixed = deploy / f"{head}.rootfs.{tail}"
    if suffixed.is_file():
        _log(f"WARNING: {name} is on disk as {suffixed.name}. A real upload would store the")
        _log("         name on disk, which would then not match zckfile in sw-description.")
        return suffixed
    globbed = _one_of(list(deploy.glob(f"{head}*.{tail}")))
    if len(globbed) == 1:
        _log(f"WARNING: serving {globbed[0].name} for {name}; names do not match")
        return globbed[0]
    sys.exit(f"cannot find {name} in {deploy} (candidates: {[p.name for p in globbed]})")


class _Artifact:
    name: str
    path: Path
    size: int
    hashes: dict[str, str]

    def __init__(self, name: str, path: Path):
        self.name = name
        self.path = path
        self.size = path.stat().st_size
        self.hashes = _hashes(path)

    def as_json(self, url: str) -> JsonDict:
        return {
            "filename": self.name,
            "hashes": self.hashes,
            "size": self.size,
            "_links": {"download": {"href": url}, "download-http": {"href": url}},
        }


def _collect(deploy: Path, swu: Path) -> tuple[list[_Artifact], str]:
    with tempfile.TemporaryDirectory() as td:
        work = Path(td)
        members = _unpack_swu(swu, work)
        desc = (work / "sw-description").read_text()

    version = _prop(desc, "version") or "0.0.0"
    zckfile = _prop(desc, "zckfile")
    if not zckfile:
        sys.exit(f"{swu.name} has no zckfile property; it is not a delta .swu")
    header = _prop(desc, "filename")
    if header and header not in members:
        sys.exit(f"sw-description names {header}, which is not in the .swu: {members}")
    url = _prop(desc, "url")
    if url != "dynamic":
        _log(f'WARNING: url is {url!r}, not "dynamic"; the handler will ignore the')
        _log("         deployment URL for the .zck. This .swu came from `pack`.")

    artifacts = [_Artifact(swu.name, swu), _Artifact(zckfile, _resolve_artifact(deploy, zckfile))]
    return artifacts, version


class State:
    deploy: Path
    artifacts: dict[str, _Artifact]
    version: str
    base_url: str
    base_path: str
    poll: int
    repeat: bool
    done: bool
    last_feedback: str
    have_config_data: bool
    lock: threading.Lock

    def __init__(
        self,
        deploy: Path,
        artifacts: list[_Artifact],
        version: str,
        base_url: str,
        tenant: str,
        device_id: str,
        poll: int,
        repeat: bool,
    ):
        self.deploy = deploy
        self.artifacts = {a.name: a for a in artifacts}
        self.version = version
        self.base_url = base_url
        self.base_path = f"{_HAWKBIT_PATH}/{tenant}/controller/v1/{device_id}"
        self.poll = poll
        self.repeat = repeat
        self.done = False
        self.last_feedback = "none"
        self.have_config_data = False
        self.lock = threading.Lock()
        self.bytes: dict[str, int] = {}
        self.statuses: dict[str, dict[str, int]] = {}
        self.ranges: dict[str, list[tuple[int, int]]] = {}

    def url_for(self, name: str) -> str:
        return f"{self.base_url}{_DOWNLOAD_PREFIX}{name}"

    def account(
        self,
        name: str,
        length: int,
        method: str,
        status: int,
        span: tuple[int, int] | None = None,
    ) -> tuple[int, int]:
        key = f"{method} {status}"
        with self.lock:
            self.bytes[name] = self.bytes.get(name, 0) + length
            outcomes = self.statuses.setdefault(name, {})
            outcomes[key] = outcomes.get(key, 0) + 1
            if span:
                self.ranges.setdefault(name, []).append(span)
            return sum(outcomes.values()), self.bytes[name]

    def summary(self) -> Summary:
        with self.lock:
            return {
                "version": self.version,
                "artifacts": {
                    name: {
                        "size": artifact.size if (artifact := self.artifacts.get(name)) else None,
                        "requests": sum(self.statuses.get(name, {}).values()),
                        "bytes": self.bytes.get(name, 0),
                        "statuses": dict(self.statuses.get(name, {})),
                        "request_ranges": list(self.ranges.get(name, [])),
                    }
                    for name in sorted(set(self.artifacts) | set(self.statuses))
                },
            }

    def report(self, summary_path: Path | None) -> None:
        _log("")
        for name in sorted(self.statuses):
            total = self.bytes[name]
            outcomes = self.statuses[name]
            artifact = self.artifacts.get(name)
            share = f" of {artifact.size}B ({100 * total / artifact.size:.1f}%)" if artifact else ""
            statuses = " ".join(f"{k}={v}" for k, v in sorted(outcomes.items()))
            _log(f"{name}: {sum(outcomes.values())} requests, {total}B{share} [{statuses}]")
            spans = self.ranges.get(name, [])
            if spans:
                # Two spans one byte apart are two chunk runs the missing-range
                # loop should have asked for as one range.
                adjacent = sum(1 for a, b in itertools.pairwise(spans) if a[1] + 1 == b[0])
                _log(f"{name}: {len(spans)} ranges, {adjacent} of them adjacent to the previous")
        if summary_path:
            with summary_path.open("w") as f:
                json.dump(self.summary(), f, indent=2, sort_keys=True)
            _log(f"wrote {summary_path}")


def _sleep_str(seconds: int) -> str:
    return f"{seconds // 3600:02d}:{seconds // 60 % 60:02d}:{seconds % 60:02d}"


class _Handler(BaseHTTPRequestHandler):
    # HTTP/1.1 for keep-alive: the delta downloader reuses one connection
    # across range requests.
    protocol_version: str = "HTTP/1.1"
    state: ClassVar[State]

    def log_message(self, format: str, *args: object) -> None:  # pyright: ignore[reportImplicitOverride]
        return

    def _route(self) -> tuple[str, str]:
        path = self.path.split("?", 1)[0]
        base = self.state.base_path
        if path == base:
            return "poll", ""
        if path == f"{base}/configData":
            return "configdata", ""
        match = re.fullmatch(re.escape(base) + r"/deploymentBase/(\d+)(/feedback)?", path)
        if match:
            return ("feedback" if match.group(2) else "deployment"), match.group(1)
        if path.startswith(_DOWNLOAD_PREFIX):
            return "download", path[len(_DOWNLOAD_PREFIX) :]
        return "unknown", path

    def _body(self) -> JsonDict:
        length = int(self.headers.get("Content-Length") or 0)
        if not length:
            return {}
        try:
            return cast("JsonDict", json.loads(self.rfile.read(length)))
        except ValueError:
            return {}

    def _send_json(self, obj: JsonDict, status: int = 200) -> None:
        body = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler dispatch name
        kind, arg = self._route()
        if kind == "poll":
            self._poll()
        elif kind == "deployment":
            self._deployment(arg)
        elif kind == "download":
            self._download(arg, head=False)
        else:
            _log(f"GET {self.path} -> 404")
            self.send_error(404)

    def do_HEAD(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler dispatch name
        kind, arg = self._route()
        if kind == "download":
            self._download(arg, head=True)
        else:
            self.send_error(404)

    def do_PUT(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler dispatch name
        kind, _arg = self._route()
        if kind != "configdata":
            self.send_error(404)
            return
        data: JsonDict = self._body().get("data") or {}
        _log(f"PUT configData {json.dumps(data, sort_keys=True)}")
        self.state.have_config_data = True
        self._send_json({})

    def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler dispatch name
        kind, arg = self._route()
        if kind != "feedback":
            self.send_error(404)
            return
        status: JsonDict = self._body().get("status") or {}
        execution = cast("str", status.get("execution", "?"))
        result: JsonDict = status.get("result") or {}
        finished = cast("str", result.get("finished", "?"))
        details = " | ".join(status.get("details") or [])
        self.state.last_feedback = f"execution={execution} finished={finished}"
        _log(f"POST feedback/{arg} execution={execution} finished={finished} {details}")
        if execution == "closed":
            if finished == "success" and not self.state.repeat:
                self.state.done = True
                _log("deployment closed successfully; no longer offering it (--repeat to keep)")
            elif finished != "success":
                _log("deployment FAILED; still offering it so the device can retry")
        self._send_json({})

    def _poll(self) -> None:
        base = f"{self.state.base_url}{self.state.base_path}"
        # Exactly one link, as the backend does: configData offered alongside
        # deploymentBase re-arms has_to_send_configData, and nothing installs.
        links: dict[str, JsonDict] = {}
        if not self.state.have_config_data:
            links["configData"] = {"href": f"{base}/configData"}
            offered = "configData"
        elif not self.state.done:
            links["deploymentBase"] = {"href": f"{base}/deploymentBase/{_ACTION_ID}"}
            offered = f"deploymentBase {_ACTION_ID}"
        else:
            offered = "no update"
        _log(f"GET poll -> {offered}")
        self._send_json({
            "config": {"polling": {"sleep": _sleep_str(self.state.poll)}},
            "_links": links,
        })

    def _deployment(self, action_id: str) -> None:
        artifacts = [a.as_json(self.state.url_for(a.name)) for a in self.state.artifacts.values()]
        _log(f"GET deploymentBase/{action_id} -> {[a['filename'] for a in artifacts]}")
        self._send_json({
            "id": action_id,
            "deployment": {
                "download": "attempt",
                "update": "attempt",
                "maintenanceWindow": "available",
                "chunks": [
                    {
                        "metadata": [],
                        "part": "default",
                        "name": "",
                        "version": self.state.version,
                        "artifacts": artifacts,
                    }
                ],
            },
            "actionHistory": {"status": "RUNNING", "messages": []},
        })

    def _download(self, name: str, *, head: bool) -> None:
        artifact = self.state.artifacts.get(name)
        path = artifact.path if artifact else (self.state.deploy / name)
        if not artifact and not (
            path.is_file() and path.resolve().is_relative_to(self.state.deploy)
        ):
            _log(f"{self.command} {name} -> 404")
            self.send_error(404)
            return

        size = path.stat().st_size
        start, end, status = 0, size - 1, 200
        rng = self.headers.get("Range")
        if rng:
            match = re.fullmatch(r"bytes=(\d*)-(\d*)", rng.strip())
            # A multi-range request gets 200 and the whole file, like BunnyCDN;
            # the delta handler treats that as fatal.
            if match:
                low, high = match.group(1), match.group(2)
                invalid_range = False
                if low == "":
                    if high == "":
                        invalid_range = True
                    else:
                        suffix_length = int(high)
                        if suffix_length <= 0:
                            invalid_range = True
                        else:
                            start = max(0, size - suffix_length)
                            end = size - 1
                else:
                    start = int(low)
                    end = min(int(high), size - 1) if high else size - 1
                    if start > end or start >= size:
                        invalid_range = True
                if invalid_range:
                    self.send_response(416)
                    self.send_header("Accept-Ranges", "bytes")
                    self.send_header("Content-Range", f"bytes */{size}")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    reqs, total = self.state.account(name, 0, self.command, 416)
                    _log(
                        f"{self.command} {name} range={rng} -> 416 unsatisfiable "
                        f"(reqs={reqs} total={total}B)"
                    )
                    return
                status = 206
            else:
                _log(f"WARNING: unsupported Range {rng!r}, answering 200 with the whole file")

        length = max(0, end - start + 1)
        self.send_response(status)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(length))
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.end_headers()

        reqs, total = self.state.account(
            name,
            0 if head else length,
            self.command,
            status,
            (start, end) if status == 206 else None,
        )
        _log(
            f"{self.command} {name} range={rng or '-'} -> {status} {length}B "
            f"(reqs={reqs} total={total}B)"
        )
        if head:
            return
        with path.open("rb") as f:
            f.seek(start)
            remaining = length
            while remaining > 0:
                block = f.read(min(1 << 16, remaining))
                if not block:
                    break
                self.wfile.write(block)
                remaining -= len(block)


def pack(deploy: Path, swu: Path, host: str, port: int) -> None:
    with tempfile.TemporaryDirectory() as td:
        work = Path(td)
        members = _unpack_swu(swu, work)

        desc = (work / "sw-description").read_text()
        zckfile = _prop(desc, "zckfile")
        if not zckfile:
            sys.exit("no zckfile property in sw-description; not a delta .swu")
        url = f"http://{host}:{port}{_DOWNLOAD_PREFIX}{zckfile}"
        patched, count = re.subn(r'url\s*=\s*"[^"]*"', f'url = "{url}"', desc)
        if count == 0:
            sys.exit("no url property in sw-description to rewrite")
        (work / "sw-description").write_text(patched)

        out = swu.with_name(swu.name[:-4] + "-local.swu")
        # sw-description must stay first; cpio -H crc matches meta-swupdate.
        order = ["sw-description"] + [m for m in members if m != "sw-description"]
        subprocess.run(
            ["cpio", "-ov", "-H", "crc", "--reproducible", "--quiet"],
            input="\n".join(order),
            text=True,
            cwd=work,
            stdout=out.open("wb"),
            check=True,
        )
    _log(f"wrote {out.name}: url -> {url}")
    _log(f"install on device: swupdate -e stable,copy<N> -i {out.name}")


def make_server(state: State, port: int) -> ThreadingHTTPServer:
    """Bind the port. Calling serve_forever() is left to the caller."""
    _Handler.state = state
    return ThreadingHTTPServer(("0.0.0.0", port), _Handler)  # noqa: S104 - the device is another host


def _run(state: State, port: int, summary_path: Path | None) -> None:
    httpd = make_server(state, port)
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        state.report(summary_path)


def serve(deploy: Path, host: str, port: int, summary_path: Path | None) -> None:
    state = State(deploy, [], "", f"http://{host}:{port}", _DEFAULT_TENANT, "", 0, True)
    _log(f"serving {deploy} on 0.0.0.0:{port} (device reaches it at http://{host}:{port}/)")
    _log("ctrl-c to stop")
    _run(state, port, summary_path)


def build_ddi_state(
    deploy: Path,
    host: str,
    port: int,
    swu: Path | None = None,
    tenant: str = _DEFAULT_TENANT,
    device_id: str = _DEFAULT_DEVICE_ID,
    poll: int = 10,
    repeat: bool = False,
) -> State:
    artifacts, version = _collect(deploy, swu or _find_swu(deploy, _DELTA_SWU_GLOB))
    base_url = f"http://{host}:{port}"
    state = State(deploy, artifacts, version, base_url, tenant, device_id, poll, repeat)

    _log(f"deployment: version {version}, poll every {poll}s")
    for artifact in artifacts:
        _log(f"  {artifact.name}  {artifact.size}B  sha1 {artifact.hashes['sha1']}")
    _log(f"suricatta url: {base_url}{_HAWKBIT_PATH}  tenant {tenant}  id {device_id}")
    _log(f"on the device, set base_url in /etc/memfaultd.conf to {base_url}")
    return state


def ddi(
    deploy: Path,
    swu: Path,
    host: str,
    port: int,
    tenant: str,
    device_id: str,
    poll: int,
    repeat: bool,
    summary_path: Path | None,
) -> None:
    state = build_ddi_state(deploy, host, port, swu, tenant, device_id, poll, repeat)
    _log("ctrl-c to stop")
    _run(state, port, summary_path)


class _Args(Protocol):
    mode: str
    deploy: Path
    swu: Path | None
    host: str
    port: int
    tenant: str
    device_id: str
    poll: int
    repeat: bool
    summary_json: Path | None


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("mode", choices=("ddi", "pack", "serve"))
    parser.add_argument("deploy", type=Path, help="deploy dir, e.g. tmp/deploy/images/qemuarm64")
    parser.add_argument(
        "--swu", type=Path, help="delta .swu to serve (default: the only one found)"
    )
    parser.add_argument(
        "--host",
        default=_DEFAULT_HOST,
        help=f"host as the device sees it (default {_DEFAULT_HOST})",
    )
    parser.add_argument("--port", type=int, default=_DEFAULT_PORT)
    parser.add_argument("--tenant", default=_DEFAULT_TENANT)
    parser.add_argument(
        "--device-id", default=os.environ.get("MEMFAULT_DEVICE_ID", _DEFAULT_DEVICE_ID)
    )
    parser.add_argument("--poll", type=int, default=10, help="polling interval to announce")
    parser.add_argument(
        "--repeat",
        action="store_true",
        help="keep offering the deployment after a successful install",
    )
    parser.add_argument(
        "--summary-json",
        type=Path,
        help="write the per-artifact request and byte counts here on exit",
    )
    args = cast("_Args", parser.parse_args())

    if not args.deploy.is_dir():
        sys.exit(f"no such directory: {args.deploy}")
    deploy = args.deploy.resolve()

    if args.mode == "serve":
        serve(deploy, args.host, args.port, args.summary_json)
        return

    swu = args.swu.resolve() if args.swu else _find_swu(deploy, _DELTA_SWU_GLOB)
    if args.mode == "pack":
        pack(deploy, swu, args.host, args.port)
    else:
        ddi(
            deploy,
            swu,
            args.host,
            args.port,
            args.tenant,
            args.device_id,
            args.poll,
            args.repeat,
            args.summary_json,
        )


if __name__ == "__main__":
    main()
