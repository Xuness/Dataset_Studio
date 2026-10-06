"""The public empty-lake command, interrupted initialization and real archive publication."""

from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import uuid

import pytest
import requests

from conftest import png
from studio_lake.archive_rebuild import build_archive
from studio_lake.updates.__main__ import dispatch
from studio_lake.updates.archive import online
from studio_lake.updates.bootstrap import create
from studio_lake.updates.sites import UpdateError
from studio_lake.updates.state import State
from studio_lake.util import IntegrityError, read_json
from test_updates import FakeSite, Images, Resources, Runner, job, post


def arguments(root, site="danbooru"):
    return dict(request_key=str(uuid.uuid4()), site=site,
                media_root=str(root / "archive"), index_root=str(root / "online"))


@pytest.fixture(autouse=True)
def offline(monkeypatch):
    def denied(*args, **kwargs):
        raise AssertionError("Empty-lake regression must not access a real source")
    monkeypatch.setattr(requests.Session, "send", denied)


@pytest.mark.parametrize("site", ["danbooru", "yandere", "gelbooru"])
def test_empty_lake_public_command_then_metadata_and_selected_media(tmp_path, site):
    state, args = State(tmp_path / "control"), arguments(tmp_path, site)
    lake = dispatch(state, "lake_create", args)
    assert create(state, args) == lake
    assert not list(tmp_path.rglob("CURRENT.json")) and not list(tmp_path.rglob("*.duckdb"))
    lib = state.library(lake["id"])
    with online(lib) as (db, status):
        assert status["served_seq"] == "0"
        assert next(db.execute("SELECT count(*) FROM objects"))[0] == 0
        assert next(db.execute("SELECT batch_id FROM publications WHERE seq=0"))[0] == "empty-baseline"
    data = png("red")
    images = Images(data)
    runner = Runner(state, {site: FakeSite(site, [post(site, p, data) for p in (11, 5000)])},
                    Resources(reserve_bytes=0), images)
    metadata = job(state, lib, dict(kind="ids", ids=[11, 5000]))
    assert runner.run(metadata["id"])["state"] == "completed"
    assert images.calls == 0
    assert lib.setting("update_new_metadata_cursor") is None
    second = job(state, lib, dict(kind="ids", ids=[5000]), "original")
    assert runner.run(second["id"])["counts"] == {"stored": 1}
    assert images.calls == 1
    with online(lib) as (db, _):
        assert list(db.execute("SELECT post_id,asset_id IS NOT NULL FROM post_versions WHERE valid_until IS NULL ORDER BY post_id")) == [(11, 0), (5000, 1)]
        before = next(db.execute("SELECT count(*) FROM observations"))[0]
    assert create(State(state.root), args) == lake
    with online(lib) as (db, _):
        assert next(db.execute("SELECT count(*) FROM observations"))[0] == before


@pytest.mark.parametrize("phase", ["archive", "online", "pointers"])
def test_interrupted_initialization_resumes_same_identity_and_generation(tmp_path, monkeypatch, phase):
    state, args = State(tmp_path / "control"), arguments(tmp_path)
    monkeypatch.setenv("DANBOORU_STORE_FAIL_AT", "after_empty_lake_" + phase)
    with pytest.raises(RuntimeError, match="injected failure"):
        create(state, args)
    intent = read_json(Path(args["media_root"]) / "initialization.json")
    monkeypatch.delenv("DANBOORU_STORE_FAIL_AT")
    result = create(State(state.root), args)
    assert result["id"] == intent["library_id"]
    assert read_json(Path(args["index_root"]) / "ONLINE.json")["generation"] == intent["generation"]
    with state.db() as db:
        assert db.execute("SELECT count(*) FROM lakes").fetchone()[0] == 1


@pytest.mark.parametrize("target", ["media_root", "index_root"])
def test_nonempty_target_is_preserved_and_cannot_be_adopted(tmp_path, target):
    state, args = State(tmp_path / "control"), arguments(tmp_path)
    path = Path(args[target])
    path.mkdir()
    owned = path / "user-data.bin"
    owned.write_bytes(b"keep this unrelated file")
    with pytest.raises(UpdateError, match="empty"):
        create(state, args)
    assert owned.read_bytes() == b"keep this unrelated file"
    assert not (Path(args["media_root"]) / "library.json").exists()


def test_parallel_replays_and_conflicting_requests(tmp_path):
    state, args = State(tmp_path / "control"), arguments(tmp_path)
    with ThreadPoolExecutor(max_workers=3) as workers:
        lakes = list(workers.map(lambda _: create(state, args), range(3)))
    assert lakes == [lakes[0]] * 3
    with pytest.raises(UpdateError, match="Request key"):
        create(state, {**args, "site": "yandere"})
    with pytest.raises(IntegrityError, match="Another initialization"):
        create(state, {**args, "request_key": str(uuid.uuid4())})


def test_recovery_does_not_recreate_a_missing_published_database(tmp_path, monkeypatch):
    state, args = State(tmp_path / "control"), arguments(tmp_path)
    monkeypatch.setenv("DANBOORU_STORE_FAIL_AT", "after_empty_lake_pointers")
    with pytest.raises(RuntimeError, match="injected failure"):
        create(state, args)
    monkeypatch.delenv("DANBOORU_STORE_FAIL_AT")
    index = Path(args["index_root"])
    pointer = read_json(index / "ONLINE.json")
    (index / pointer["file"]).unlink()
    with pytest.raises(IntegrityError, match="missing"):
        create(state, args)


def test_new_archive_identity_survives_independent_rebuild(tmp_path):
    state, args = State(tmp_path / "control"), arguments(tmp_path, "yandere")
    lake = create(state, args)
    with pytest.raises(IntegrityError, match="站点"):
        build_archive(args["media_root"], tmp_path / "wrong-site", "danbooru")
    rebuilt = build_archive(args["media_root"], tmp_path / "rebuild", "yandere")
    assert rebuilt["library_id"] == lake["id"]
    assert rebuilt["counts"] == dict(objects=0, observations=0, assets=0)
