#
# Copyright (c) Memfault, Inc.
# See License.txt for details
import dataclasses
import datetime
import time
import uuid
from collections.abc import Callable, Mapping, Sequence
from typing import Literal, TypeAlias, TypeVar, cast
from unittest.mock import ANY

import requests

Json: TypeAlias = Mapping[str, "Json"] | Sequence["Json"] | str | int | float | bool | None

_T = TypeVar("_T")


@dataclasses.dataclass
class MemfaultServiceTester:
    base_url: str
    organization_slug: str
    project_slug: str
    organization_token: str

    session: requests.Session = dataclasses.field(init=False)

    def __post_init__(self) -> None:
        self.base_url = self.base_url.rstrip("/")
        self.session = requests.Session()
        self.session.auth = ("", self.organization_token)

    @property
    def _api_url(self) -> str:
        return f"{self.base_url}/api/v0"

    @property
    def _project_url(self) -> str:
        return (
            f"{self._api_url}/organizations/{self.organization_slug}/projects/{self.project_slug}"
        )

    def poll_until_not_raising(
        self,
        check_callback: Callable[[], _T],
        timeout_seconds: float = 10,
        poll_interval_seconds: float = 0.5,
    ) -> _T:
        timeout = time.time() + timeout_seconds
        while True:
            try:
                rv = check_callback()
            except Exception:
                if time.time() > timeout:
                    raise
            else:
                return rv
            time.sleep(poll_interval_seconds)

    def list_reboot_events(
        self,
        *,
        device_serial: str,
        params: Json = None,
        expect_status: int = 200,
    ) -> list[dict[str, Json]] | None:
        if params is None:
            params = {}

        url = f"{self._project_url}/devices/{device_serial}/reboots"
        resp = self.session.get(
            url,
            params=params,  # type: ignore[arg-type]
        )
        assert resp.status_code == expect_status
        return cast(list[dict[str, Json]], resp.json()["data"]) if resp.ok else None

    def poll_reboot_events_until_count(
        self,
        count: int,
        device_serial: str,
        params: Json = None,
        timeout_secs: int = 10,
    ) -> list[dict[str, Json]]:
        def _check() -> list[dict[str, Json]]:
            events = self.list_reboot_events(
                device_serial=device_serial, params=params, expect_status=ANY
            )
            assert events is not None
            assert len(events) >= count
            events.sort(key=lambda x: datetime.datetime.fromisoformat(cast(str, x["time"])))
            return events

        return self.poll_until_not_raising(_check, timeout_seconds=timeout_secs)

    def list_reports(
        self,
        params: Json = None,
        expect_status: int = 200,
        ignore_errors: bool = False,
    ) -> list[dict[str, Json]]:
        rv = self.session.get(
            f"{self._project_url}/reports",
            params=params,  # type: ignore[arg-type]
        )
        assert rv.status_code == expect_status or ignore_errors
        return cast(list[dict[str, Json]], rv.json()["data"]) if rv.status_code == 200 else []

    def poll_reports_until_count(
        self,
        count: int,
        device_serial: str | None = None,
        params: dict[str, Json] | None = None,
        timeout_secs: int = 10,
    ) -> list[dict[str, Json]]:
        if not params:
            params = {}
        if device_serial:
            params = {"device_serial": device_serial, **params}

        num_reports = 0
        timeout = time.time() + timeout_secs
        reports = []
        while True:
            if num_reports >= count or time.time() > timeout:
                assert num_reports >= count, f"Got: {num_reports}, Expected: {count}"
                break
            reports = self.list_reports(
                params,
                ignore_errors=True,
            )
            num_reports = len(reports)
            time.sleep(0.5)
        return reports

    def list_elf_coredumps(
        self,
        params: Json = None,
        expect_status: int = 200,
        ignore_errors: bool = False,
    ) -> list[dict[str, Json]]:
        rv = self.session.get(
            f"{self._project_url}/elf_coredumps",
            params=params,  # type: ignore[arg-type]
        )
        assert rv.status_code == expect_status or ignore_errors
        return cast(list[dict[str, Json]], rv.json()["data"]) if rv.status_code == 200 else []

    def poll_elf_coredumps_until_count(
        self,
        count: int,
        device_serial: str | None = None,
        params: dict[str, Json] | None = None,
        timeout_secs: int = 10,
    ) -> list[dict[str, Json]]:
        if not params:
            params = {}
        if device_serial:
            params = {"device": device_serial, **params}

        def _check() -> list[dict[str, Json]]:
            elf_coredumps = self.list_elf_coredumps(params=params, expect_status=ANY)
            assert len(elf_coredumps) >= count
            return elf_coredumps

        return self.poll_until_not_raising(_check, timeout_seconds=timeout_secs)

    def list_attributes(
        self,
        *,
        device_serial: str,
        params: Json = None,
        expect_status: int = 200,
    ) -> list[dict[str, Json]] | None:
        if params is None:
            params = {}

        url = f"{self._project_url}/devices/{device_serial}/attributes"
        resp = self.session.get(
            url,
            params=params,  # type: ignore[arg-type]
        )
        assert resp.status_code == expect_status
        return cast(list[dict[str, Json]], resp.json()["data"]) if resp.ok else None

    def patch_device_attributes(
        self,
        *,
        device_serial: str,
        patch: dict[str, bool | int | float | str],
        expect_status: int = 204,
    ) -> None:
        url = f"{self._project_url}/devices/{device_serial}/attributes"
        resp = self.session.patch(
            url, json=[{"string_key": k, "value": v} for k, v in patch.items()]
        )
        assert resp.status_code == expect_status, resp.json()

    def create_custom_metric(
        self,
        *,
        key: str,
        data_type: Literal["INT", "FLOAT", "STRING", "BOOL"],
        expect_status: int = 200,
    ) -> dict[str, Json] | None:
        url = f"{self._project_url}/custom-metrics"
        resp = self.session.post(
            url,
            json={
                "string_key": key,
                "data_type": data_type,
            },
        )
        assert resp.status_code == expect_status, resp.json()
        return cast(dict[str, Json], resp.json()["data"]) if resp.ok else None

    def log_files_get_list(
        self, device_serial: str, params: dict[str, Json] | None = None
    ) -> list[dict[str, Json]]:
        rv = self.session.get(
            f"{self._project_url}/devices/{device_serial}/log-files",
            params=params,  # type: ignore[arg-type]
        )
        assert rv.status_code == 200
        return cast(list[dict[str, Json]], rv.json()["data"])

    def log_file_download(self, device_serial: str, cid: str | uuid.UUID) -> str:
        rv = self.session.get(
            f"{self._project_url}/devices/{device_serial}/log-files/{cid}/download"
        )
        assert rv.status_code == 200
        return rv.text

    def get_device(self, device_serial: str) -> dict[str, Json]:
        rv = self.session.get(f"{self._project_url}/devices/{device_serial}")
        assert rv.status_code == 200, f"Get device failed with status: {rv.status_code}"
        return cast(dict[str, Json], rv.json()["data"])
