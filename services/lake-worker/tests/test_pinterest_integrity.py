"""Raw byte evidence and immutable pack copies are verified independently of rendering."""

import hashlib
import zlib

import pytest

from pinterest_fixtures import Fixture
from studio_lake.archive_rebuild import build_archive, verify_archive, compare_reference
from studio_lake.pinterest.http import Response
from studio_lake.pinterest.lake import library
from studio_lake.canonical import utc
from studio_lake.util import IntegrityError


def test_non_utf8_response_remains_rebuildable_and_byte_exact(tmp_path):
    fixture = Fixture(tmp_path)
    body = b"<html>\xff\xfe\x80source error page</html>"

    def factory(root, context, *, cancelled):
        class Client:
            def pin(self, identity):
                self.before_send()
                return Response(body, "/resource/PinResource/get/", {}, 200, utc(), context)

            def close(self):
                pass
        return Client()

    fixture.factory = factory
    assert fixture.run()["state"] == "needs_review"
    output = tmp_path / "rebuilt"
    assert build_archive(tmp_path / "media", output, "pinterest", reference_index=tmp_path / "index")["state"] == "built"
    assert verify_archive(output)["raw_roundtrip_verified"]
    assert compare_reference(output)["equal"]
    import sqlite3
    with sqlite3.connect(output / "online.sqlite") as db:
        compressed, size, sha = db.execute("SELECT raw_zlib,raw_bytes,raw_sha256 FROM captures").fetchone()
    assert zlib.decompress(compressed) == body and size == len(body) and sha == hashlib.sha256(body).hexdigest()


def test_corrupted_tar_copy_never_enters_the_journal(tmp_path, monkeypatch):
    fixture = Fixture(tmp_path)
    copy = library.shutil.copyfile

    def corrupt(source, target, *args, **kwargs):
        value = copy(source, target, *args, **kwargs)
        if target.name == "media.tar":
            with target.open("r+b") as file:
                file.seek(512)
                value_at = file.read(1)
                file.seek(512)
                file.write(bytes([value_at[0] ^ 1]))
        return value

    monkeypatch.setattr(library.shutil, "copyfile", corrupt)
    with pytest.raises(IntegrityError, match="copy|Copy"):
        fixture.run()
    lib = fixture.service.library(fixture.lake["library_id"])
    assert lib.verify(deep=True) == dict(batches=2, objects=0, captures=1)
    monkeypatch.setattr(library.shutil, "copyfile", copy)
    row = fixture.service.job(fixture.job["id"])
    fixture.service.action(dict(job_id=row["id"], action="resume", expected_revision=row["revision"]))
    assert fixture.run()["state"] == "completed"
    assert len(fixture.media_calls) == 1 and lib.verify(deep=True)["objects"] == 1
