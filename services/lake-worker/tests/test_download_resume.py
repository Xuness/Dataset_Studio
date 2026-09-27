import hashlib
import json
import threading
from pathlib import Path
from types import SimpleNamespace

import pytest
import requests

from studio_lake.updates import transfer
from studio_lake.updates.media import ImageSessions
from studio_lake.updates.resources import Resources
from studio_lake.updates.sites import UpdateError


DATA = bytes(range(251)) * 12000
PREFIX = 256 * 1024
URL = "https://files.yande.re/test-original.png"


class Response:
    def __init__(self, data=DATA, status=200, headers=None, fail=False, hook=None):
        self.data, self.status_code, self.headers = data, status, headers or {}
        self.fail, self.hook = fail, hook

    def __enter__(self):
        return self

    def __exit__(self, *_):
        pass

    def iter_content(self, size):
        for i in range(0, len(self.data), size):
            yield self.data[i : i + size]
            if self.hook:
                self.hook()
            if self.fail:
                raise requests.exceptions.ConnectionError("fixture secret-url?api_key=never-report-me")


class HTTP:
    def __init__(self, responses):
        self.responses, self.calls = list(responses), []

    def get(self, url, **kw):
        self.calls.append((url, kw))
        return self.responses.pop(0)


def fetch(tmp_path, http, *, url=URL, kind="original", md5=True, cancelled=lambda: False):
    values = []
    observation = {"post_id": 11, "md5": hashlib.md5(DATA).hexdigest() if md5 else None}
    result = transfer.fetch(
        tmp_path,
        "test",
        url,
        kind,
        observation,
        SimpleNamespace(name="yandere", rate_root=None),
        Resources(reserve_bytes=0),
        cancelled,
        ImageSessions("yandere", http),
        lambda **v: values.append(v),
    )
    return result, values


def interrupted(tmp_path, *, md5=True, etag='"original-v1"'):
    headers = {"Content-Length": str(len(DATA))}
    if etag:
        headers["ETag"] = etag
    response = Response(headers=headers, fail=True)
    result, _ = fetch(tmp_path, HTTP([response]), md5=md5)
    assert result["state"] == "failed"
    assert (tmp_path / "test.partial").read_bytes() == DATA[:PREFIX]
    assert json.loads((tmp_path / "test.partial.json").read_text())["bytes"] == PREFIX
    return result


def ranged(data=DATA[PREFIX:], *, start=PREFIX, end=None, total=len(DATA), etag='"original-v1"'):
    return Response(
        data,
        206,
        {
            "Content-Range": f"bytes {start}-{end if end is not None else total - 1}/{total}",
            "Content-Length": str(len(data)),
            "ETag": etag,
        },
    )


def test_broken_transfer_resumes_validated_range_and_counts_only_new_bytes(tmp_path):
    first = interrupted(tmp_path)
    assert first["transfer_error"]["resumable_bytes"] == PREFIX
    assert "secret" not in json.dumps(first)
    http = HTTP([ranged()])
    result, progress = fetch(tmp_path, http)
    assert result["state"] == "downloaded" and result["original_md5_verified"]
    assert Path(result["download_path"]).read_bytes() == DATA
    assert http.calls[0][1]["headers"] == {
        "Accept-Encoding": "identity",
        "Range": f"bytes={PREFIX}-",
        "If-Range": '"original-v1"',
    }
    assert sum(v.get("downloaded_bytes_delta", 0) for v in progress) == len(DATA) - PREFIX
    assert sum(v.get("resumed_requests_delta", 0) for v in progress) == 1
    assert not (tmp_path / "test.partial.json").exists()


@pytest.mark.parametrize(
    "kind,md5,etag,resume",
    [
        ("original", True, None, True),
        ("original", False, '"strong"', True),
        ("sample", False, '"strong"', True),
        ("original", False, None, False),
        ("original", False, 'W/"weak"', False),
    ],
)
def test_resume_requires_source_hash_or_strong_validator(tmp_path, kind, md5, etag, resume):
    headers = {"Content-Length": str(len(DATA)), "ETag": etag or ""}
    fetch(tmp_path, HTTP([Response(headers=headers, fail=True)]), kind=kind, md5=md5)
    http = HTTP([ranged(etag=etag) if resume else Response()])
    result, _ = fetch(tmp_path, http, kind=kind, md5=md5)
    assert result["state"] == "downloaded"
    assert ("Range" in http.calls[0][1]["headers"]) == resume
    assert Path(result["download_path"]).read_bytes() == DATA


def test_server_ignoring_range_replaces_file_without_appending_old_prefix(tmp_path):
    interrupted(tmp_path)
    result, progress = fetch(tmp_path, HTTP([Response(headers={"Content-Length": str(len(DATA))})]))
    assert Path(result["download_path"]).read_bytes() == DATA
    assert sum(v.get("resumed_requests_delta", 0) for v in progress) == 0
    assert sum(v.get("downloaded_bytes_delta", 0) for v in progress) == len(DATA)


@pytest.mark.parametrize(
    "response",
    [
        ranged(start=PREFIX + 1),
        ranged(total=len(DATA) + 1),
        ranged(etag='"changed"'),
        Response(DATA[PREFIX:], 206, {"Content-Range": "bytes invalid"}),
        Response(
            DATA[PREFIX:],
            206,
            {"Content-Range": f"bytes {PREFIX}-{len(DATA) - 1}/{len(DATA)}", "Content-Length": "1"},
        ),
        Response(
            DATA[PREFIX:],
            206,
            {"Content-Range": f"bytes {PREFIX}-{len(DATA) - 1}/{len(DATA)}", "Content-Encoding": "gzip"},
        ),
    ],
)
def test_mismatched_range_or_representation_is_never_published(tmp_path, response):
    interrupted(tmp_path)
    result, _ = fetch(tmp_path, HTTP([response]))
    assert result["reason"] == "image_range_invalid"
    assert not (tmp_path / "test.downloaded").exists()
    assert not (tmp_path / "test.partial").exists()


def test_416_restarts_once_and_does_not_treat_truncated_file_as_complete(tmp_path):
    interrupted(tmp_path)
    http = HTTP([Response(b"", 416, {"Content-Range": f"bytes */{PREFIX}"}), Response()])
    result, _ = fetch(tmp_path, http)
    assert result["state"] == "downloaded"
    assert len(http.calls) == 2 and "Range" not in http.calls[1][1]["headers"]
    assert Path(result["download_path"]).read_bytes() == DATA


@pytest.mark.parametrize("change", ["corrupt", "url", "receipt"])
def test_checkpoint_identity_or_hash_mismatch_safely_restarts(tmp_path, change):
    interrupted(tmp_path)
    if change == "corrupt":
        (tmp_path / "test.partial").write_bytes(b"x" * PREFIX)
    if change == "receipt":
        (tmp_path / "test.partial.json").write_text("{broken", encoding="utf-8")
    http = HTTP([Response()])
    result, _ = fetch(tmp_path, http, url=URL + "?changed" if change == "url" else URL)
    assert "Range" not in http.calls[0][1]["headers"]
    assert Path(result["download_path"]).read_bytes() == DATA


def test_uncheckpointed_crash_tail_is_truncated_before_range_request(tmp_path):
    interrupted(tmp_path)
    abandoned = tmp_path / "test.partial.json.abandoned.tmp"
    abandoned.write_text('{"uncommitted": true}')
    with (tmp_path / "test.partial").open("ab") as output:
        output.write(b"incomplete-uncommitted-tail")
    http = HTTP([ranged()])
    result, _ = fetch(tmp_path, http)
    assert http.calls[0][1]["headers"]["Range"] == f"bytes={PREFIX}-"
    assert Path(result["download_path"]).read_bytes() == DATA
    assert not abandoned.exists()


def test_complete_hash_recovers_crash_before_eof_receipt_without_an_extra_request(tmp_path, monkeypatch):
    original = transfer.Partial.checkpoint

    def crash(self, output, *, complete=False):
        if complete:
            raise RuntimeError("crash before EOF receipt")
        return original(self, output, complete=complete)

    with monkeypatch.context() as patch:
        patch.setattr(transfer.Partial, "checkpoint", crash)
        with pytest.raises(RuntimeError, match="EOF"):
            fetch(tmp_path, HTTP([Response()]))
    http = HTTP([])
    result, _ = fetch(tmp_path, http)
    assert not http.calls and Path(result["download_path"]).read_bytes() == DATA


def test_pause_retains_partial_and_complete_checkpoint_needs_no_network(tmp_path, monkeypatch):
    cancelled = threading.Event()
    response = Response(headers={"Content-Length": str(len(DATA))}, hook=cancelled.set)
    with pytest.raises(UpdateError, match="paused"):
        fetch(tmp_path, HTTP([response]), cancelled=cancelled.is_set)
    assert (tmp_path / "test.partial").stat().st_size == PREFIX
    original_finish = transfer.Partial.finish
    with monkeypatch.context() as patch:
        patch.setattr(
            transfer.Partial, "finish", lambda _self: (_ for _ in ()).throw(RuntimeError("kill window"))
        )
        with pytest.raises(RuntimeError, match="kill window"):
            fetch(tmp_path, HTTP([ranged(etag=None)]))
    http = HTTP([])
    result, _ = fetch(tmp_path, http)
    assert transfer.Partial.finish is original_finish and not http.calls
    assert Path(result["download_path"]).read_bytes() == DATA


def test_resume_final_md5_mismatch_never_produces_a_raw_receipt(tmp_path):
    interrupted(tmp_path)
    result, _ = fetch(tmp_path, HTTP([ranged(data=b"x" * (len(DATA) - PREFIX))]))
    assert result["reason"] == "original_md5_mismatch"
    assert not (tmp_path / "test.download.json").exists()
    assert not (tmp_path / "test.partial").exists()


def test_full_response_shorter_than_content_length_is_resumable(tmp_path):
    result, _ = fetch(tmp_path, HTTP([Response(DATA[:PREFIX], headers={"Content-Length": str(len(DATA))})]))
    assert result["state"] == "failed"
    assert result["transfer_error"]["resumable_bytes"] == PREFIX
    assert not (tmp_path / "test.downloaded").exists()
    result, _ = fetch(tmp_path, HTTP([ranged(etag=None)]))
    assert Path(result["download_path"]).read_bytes() == DATA
