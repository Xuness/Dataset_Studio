"""Physical packing keeps logical identities, recovery boundaries, and bounded staging."""

import io
import os
from pathlib import Path
import subprocess
import sys
import uuid

from PIL import Image
import pytest

from pinterest_fixtures import Fixture, MediaResponse
from studio_lake import archive_io
from studio_lake.online_storage import connect
from studio_lake.pinterest import outbox, packing, receipts, runner
from studio_lake.pinterest.lake import FEATURES, library, online
from studio_lake.updates.resources import Resources
from studio_lake.updates.state import SCHEMA_VERSION, State
from studio_lake.util import IntegrityError, atomic_json, file_hash, read_json


class Distinct(Fixture):
    def __init__(self, root, count=100, **kwargs):
        super().__init__(root, tuple(str(1000 + i) for i in range(count)),
                         pin_transform=lambda p: p["images"]["orig"].update(url=p["images"]["orig"]["url"] + "?pin=" + p["id"]), **kwargs)

    def get(self, url, **kwargs):
        self.media_calls.append((url, kwargs))
        n = int(url.rsplit("=", 1)[-1])
        stream = io.BytesIO()
        Image.new("RGB", (8, 6), (n % 256, n // 256 % 256, 0)).save(stream, format="PNG")
        return MediaResponse(stream.getvalue())


class Crash(BaseException):
    pass


def sql(fixture, statement, args=()):
    with fixture.state.db() as db:
        return db.execute(statement, args).fetchall()


def test_hundred_pins_use_bounded_multi_object_packs_and_rebuild(tmp_path):
    fixture = Distinct(tmp_path)
    execution = fixture.runner()
    result = execution.run(fixture.job["id"], time_slice=180)
    assert result["state"] == "completed", result
    assert len(fixture.pin_calls) == len(fixture.media_calls) == 100
    lib = fixture.service.library(fixture.lake["library_id"])
    assert result["archive_seq"] == result["served_seq"] == 5
    assert result["metrics"]["new_byte_objects"] == 100
    assert len(list((lib.root / "segments").glob("*/media.tar"))) == 2
    assert lib.verify(deep=True) == dict(batches=5, captures=100, objects=100)
    with lib.journal() as db:
        groups = db.execute("SELECT seq,count(*) FROM pinterest_receipts GROUP BY seq ORDER BY seq").fetchall()
        assert [r[1] for r in groups] == [1, 64, 64, 36, 36]
    assert execution.resources.snapshot()["staging_reserved_bytes"] == 0
    assert not execution.resources.retained
    assert sql(fixture, "SELECT count(*) FROM pinterest_outbox WHERE cleanup_complete=0")[0][0] == 0
    rebuilt = online.rebuild(library.PinterestLibrary.archive(lib.root), tmp_path / "rebuilt")
    assert rebuilt["served_seq"] == 5
    db = connect(tmp_path / "rebuilt" / "online.sqlite")
    try:
        assert db.execute("SELECT count(*) FROM assets").fetchone()[0] == 100
    finally:
        db.close()


@pytest.mark.parametrize("point", ["pinterest_after_outbox", "pinterest_after_freeze", "pinterest_during_pack",
    "pinterest_after_seal", "pinterest_after_rename", "pinterest_after_commit", "pinterest_after_publish",
    "pinterest_after_control_replay", "pinterest_during_cleanup"])
def test_multi_member_crashes_preserve_frozen_members_and_requests(tmp_path, monkeypatch, point):
    fixture = Distinct(tmp_path, count=4)
    fired = []
    def fault(name):
        if name == point and len(fixture.media_calls) >= 4 and not fired:
            fired.append(name)
            raise Crash()
    for module in (outbox, packing, runner, archive_io, library, online):
        monkeypatch.setattr(module, "failpoint", fault)
    with pytest.raises(Crash):
        fixture.run()
    frozen = sql(fixture, "SELECT batch_id,receipt_ids_json,dedupe_key FROM pinterest_batches ORDER BY batch_id")
    execution = fixture.runner()
    result = execution.run(fixture.job["id"], time_slice=60)
    assert result["state"] == "completed", result
    assert len(fixture.pin_calls) == len(fixture.media_calls) == 4
    assert set(frozen) <= set(sql(fixture, "SELECT batch_id,receipt_ids_json,dedupe_key FROM pinterest_batches"))
    assert result["archive_seq"] == result["served_seq"] == 3
    assert fixture.service.library(fixture.lake["library_id"]).verify(deep=True) == dict(batches=3, captures=4, objects=4)
    assert execution.resources.snapshot()["staging_reserved_bytes"] == 0


def buffer_three(fixture, monkeypatch):
    execution = fixture.runner()
    def stop(name):
        if name == "pinterest_after_prepared" and len(fixture.pin_calls) == 3:
            execution.stop.set()
    monkeypatch.setattr(runner, "failpoint", stop)
    result = execution.run(fixture.job["id"], time_slice=60)
    assert result["state"] == "queued" and result["archive_seq"] == 1
    assert result["metrics"]["buffered_results"] == 3 and not fixture.media_calls
    return execution


@pytest.mark.parametrize("action", ["resume", "cancel"])
def test_pending_results_survive_slice_restart_pause_and_cancel(tmp_path, monkeypatch, action):
    fixture = Distinct(tmp_path, count=6)
    previous = buffer_three(fixture, monkeypatch)
    assert previous.resources.snapshot()["staging_reserved_bytes"] > 0
    execution = fixture.runner()
    snapshot = execution.resources.snapshot()
    assert snapshot["staging_reserved_bytes"] > snapshot["staging_bytes"] > 0
    job = fixture.service.job(fixture.job["id"])
    assert job["totals"].get("admitted_pins", 0) == 0 and job["budget_usage"]["admitted_pins"] == 3
    fixture.service.action(dict(job_id=job["id"], action="pause", expected_revision=job["revision"]))
    paused = execution.run(job["id"])
    assert paused["state"] == "paused" and paused["archive_seq"] == 1
    monkeypatch.setattr(runner, "failpoint", lambda _: None)
    fixture.service.action(dict(job_id=job["id"], action=action, expected_revision=paused["revision"]))
    result = execution.run(job["id"], time_slice=60)
    assert result["state"] == ("completed" if action == "resume" else "cancelled")
    assert len(fixture.pin_calls) == (6 if action == "resume" else 3)
    assert len(fixture.media_calls) == (6 if action == "resume" else 0)
    assert execution.resources.snapshot()["staging_reserved_bytes"] == 0


def test_pending_admissions_honor_budget_before_publication(tmp_path):
    fixture = Distinct(tmp_path, count=6, budget=dict(admitted_pins=3))
    result = fixture.run()
    assert result["state"] == "waiting_budget" and len(fixture.pin_calls) == 3
    assert result["totals"]["admitted_pins"] == 3 and len(fixture.media_calls) == 3
    fixture.service.action(dict(job_id=result["id"], action="continue", expected_revision=result["revision"]))
    assert fixture.run()["state"] == "completed"
    assert len(fixture.pin_calls) == len(fixture.media_calls) == 6


def test_replay_pagination_and_failure_are_atomic_for_the_whole_physical_group(tmp_path, monkeypatch):
    monkeypatch.setattr(outbox, "MAX_RECEIPTS", 24)
    fixture = Distinct(tmp_path, count=70)
    result = fixture.runner().run(fixture.job["id"], time_slice=180)
    assert result["state"] == "completed"
    lib = fixture.service.library(fixture.lake["library_id"])
    with fixture.state.db() as db:
        db.execute("DELETE FROM pinterest_applied WHERE seq>1")
        db.execute("DELETE FROM pinterest_metrics")
        db.execute("UPDATE pinterest_jobs SET archive_seq=1")
        db.execute("UPDATE pinterest_tasks SET state='prepared'")
    original, calls = receipts._apply, []
    def fail_member(db, row, job_id):
        original(db, row, job_id)
        calls.append(row["receipt_id"])
        if len(calls) == 120:
            raise Crash()
    monkeypatch.setattr(receipts, "_apply", fail_member)
    with pytest.raises(Crash):
        receipts.replay(fixture.state, lib, fixture.job["id"])
    assert sql(fixture, "SELECT count(*) FROM pinterest_tasks WHERE state='done'")[0][0] == 118
    assert fixture.service.row(fixture.job["id"])["archive_seq"] == 6
    monkeypatch.setattr(receipts, "_apply", original)
    receipts.replay(fixture.state, lib, fixture.job["id"])
    assert sql(fixture, "SELECT count(*) FROM pinterest_tasks WHERE state='done'")[0][0] == 140
    assert sql(fixture, "SELECT count(*) FROM pinterest_applied")[0][0] == 141
    assert fixture.service.row(fixture.job["id"])["archive_seq"] == result["archive_seq"]


def test_legacy_prepared_and_archives_mix_without_rewriting_old_segments(tmp_path, monkeypatch):
    fixture = Fixture(tmp_path, ("123",))
    lib = fixture.service.library(fixture.lake["library_id"])
    info = {**lib.info, "required_features": list(FEATURES)}
    atomic_json(lib.root / "library.json", info)
    save = receipts.save_prepared
    monkeypatch.setattr(receipts, "save_prepared", lambda root, replay, records, media=None, **_: save(root, replay, records, media))
    assert fixture.run()["state"] == "completed"
    hashes = {str(p.relative_to(lib.root)): file_hash(p) for p in (lib.root / "segments").glob("*/*") if p.is_file()}
    pointer = read_json(lib.cache / "ONLINE.json")
    monkeypatch.setattr(receipts, "save_prepared", save)
    fixture.job = fixture.service.create(dict(request_key=str(uuid.uuid4()), definition={**fixture.spec, "seeds": [dict(kind="pin", id="124")]}))
    assert fixture.run()["state"] == "completed"
    assert hashes == {name: file_hash(lib.root / name) for name in hashes}
    assert read_json(lib.cache / "ONLINE.json") == pointer
    lib = fixture.service.library(fixture.lake["library_id"])
    assert lib.verify(deep=True)["captures"] == 2
    assert "receipt-bundles-v1" in lib.info["required_features"]
    atomic_json(lib.root / "library.json", {**lib.info, "required_features": [*lib.info["required_features"], "unknown-feature"]})
    with pytest.raises(IntegrityError, match="features"):
        fixture.service.library(fixture.lake["library_id"])


def test_v21_upgrade_preserves_claims_and_creates_backup(tmp_path):
    from update_fixtures import remove_pinterest_batches
    fixture = Fixture(tmp_path)
    execution = fixture.runner()
    lib = fixture.service.library(fixture.lake["library_id"])
    execution._intent(lib, fixture.service.row(fixture.job["id"]))
    task = execution._claim(fixture.job["id"], fixture.job["definition"])
    with fixture.state.db() as db:
        remove_pinterest_batches(db)
        db.execute("PRAGMA user_version=21")
    upgraded = State(fixture.state.root)
    with upgraded.db() as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == SCHEMA_VERSION
        assert tuple(db.execute("SELECT state,claim_token,receipt_id FROM pinterest_tasks WHERE task_id=?", (task["task_id"],)).fetchone()) == ("running", task["claim_token"], task["receipt_id"])
        assert db.execute("SELECT count(*) FROM pinterest_outbox").fetchone()[0] == 0
    assert len(list((fixture.state.root / "backups").glob("control-v21-*.sqlite"))) == 1


def test_changed_durable_prepared_result_fails_closed_without_refetch(tmp_path, monkeypatch):
    fixture = Distinct(tmp_path, count=3)
    buffer_three(fixture, monkeypatch)
    root = tmp_path / "index" / "pinterest_receipts" / fixture.job["id"]
    path = next(root.glob("*.prepared.json"))
    value = read_json(path)
    value["replay"]["metrics"] = dict(tampered=1)
    atomic_json(path, value)
    monkeypatch.setattr(runner, "failpoint", lambda _: None)
    with pytest.raises(IntegrityError, match="changed"):
        fixture.run()
    assert len(fixture.pin_calls) == 3 and not fixture.media_calls
    assert fixture.service.job(fixture.job["id"])["state"] == "needs_review"


def test_buffer_age_flushes_before_admitting_another_request(tmp_path, monkeypatch):
    fixture = Distinct(tmp_path, count=6)
    buffer_three(fixture, monkeypatch)
    monkeypatch.setattr(runner, "failpoint", lambda _: None)
    with fixture.state.db() as db:
        db.execute("UPDATE pinterest_outbox SET created_at=created_at-121")
    execution = fixture.runner()
    monkeypatch.setattr(packing, "failpoint", lambda name: execution.stop.set() if name == "pinterest_after_control_replay" else None)
    result = execution.run(fixture.job["id"], time_slice=60)
    assert result["archive_seq"] == 2 and len(fixture.pin_calls) == 3 and not fixture.media_calls
    assert result["metrics"]["buffered_results"] == 0


def test_lowered_spool_limit_keeps_existing_debt_and_waits_without_fetching(tmp_path, monkeypatch):
    fixture = Distinct(tmp_path, count=6)
    buffer_three(fixture, monkeypatch)
    monkeypatch.setattr(runner, "failpoint", lambda _: None)
    resources = Resources(spool_bytes=64 * 1024, reserve_bytes=0)
    execution = runner.Runner(fixture.state, resources=resources, client_factory=fixture.factory, image_http=fixture)
    assert resources.snapshot()["staging_reserved_bytes"] > resources.spool_bytes
    result = execution.run(fixture.job["id"])
    assert result["state"] == "waiting_resources"
    assert len(fixture.pin_calls) == 3 and not fixture.media_calls
    resources.overrides["spool_bytes"] = 64 * 1024**2
    assert execution.run(fixture.job["id"], time_slice=60)["state"] == "completed"
    assert len(fixture.pin_calls) == len(fixture.media_calls) == 6


def test_sampler_does_not_schedule_more_than_its_frozen_quota(tmp_path):
    from test_pinterest_discovery import collection, listed, pages
    fixture = collection(tmp_path, lambda *_: pages([listed(str(1000+i)) for i in range(25)]), metadata="sample")
    result = fixture.run()
    assert result["state"] == "completed", result
    assert result["metrics"]["sampled_manifests"] == 3
    assert len(fixture.pin_calls) == 3


@pytest.mark.parametrize("target_mib", [7, 128])
def test_low_spool_flushes_funded_pending_media_and_allows_legal_single_oversize(tmp_path, monkeypatch, target_mib):
    class Larger(Distinct):
        def get(self, url, **kwargs):
            response = super().get(url, **kwargs)
            return MediaResponse(response.data + b"\0" * (8 * 1024**2))
    fixture = Larger(tmp_path, count=6)
    resources = Resources(max_download_bytes=16 * 1024**2, spool_bytes=40 * 1024**2, reserve_bytes=0)
    snapshots, resize = [], resources._resize
    def measured(*args, **kwargs):
        result = resize(*args, **kwargs)
        snapshots.append(resources.snapshot()["staging_reserved_bytes"])
        return result
    monkeypatch.setattr(resources, "_resize", measured)
    # The target is soft for a single legal image; hard spool/decode/download limits still apply.
    monkeypatch.setattr(outbox, "MAX_MEDIA", target_mib * 1024**2)
    execution = runner.Runner(fixture.state, resources=resources, client_factory=fixture.factory, image_http=fixture)
    result = execution.run(fixture.job["id"], time_slice=180)
    assert result["state"] == "completed", result
    assert len(fixture.media_calls) == 6 and result["metrics"]["new_byte_objects"] == 6
    assert max(snapshots) <= resources.spool_bytes
    assert resources.snapshot()["staging_reserved_bytes"] == 0
    assert len(list((tmp_path / "media" / "segments").glob("*/media.tar"))) == (6 if target_mib == 7 else 3)


def test_restart_restores_pack_promises_before_other_lakes_can_admit(tmp_path, monkeypatch):
    fixture = Distinct(tmp_path, count=4)
    def fault(name):
        if name == "pinterest_during_pack":
            raise Crash()
    monkeypatch.setattr(packing, "failpoint", fault)
    with pytest.raises(Crash):
        fixture.run()
    resources = Resources(spool_bytes=32 * 1024**2, reserve_bytes=0)
    execution = runner.Runner(fixture.state, resources=resources, client_factory=fixture.factory, image_http=fixture)
    snapshot = resources.snapshot()
    assert snapshot["staging_reserved_bytes"] > snapshot["staging_bytes"]
    other = resources.try_reserve(tmp_path / "another-lake" / "job", "other", resources.spool_bytes - snapshot["staging_reserved_bytes"])
    assert other is not None
    monkeypatch.setattr(packing, "failpoint", lambda _: None)
    try:
        assert execution.run(fixture.job["id"], time_slice=60)["state"] == "completed"
        assert len(fixture.pin_calls) == len(fixture.media_calls) == 4
    finally:
        other.release()
    assert resources.snapshot()["staging_reserved_bytes"] == 0


def test_same_bytes_and_shared_url_metrics_count_once_inside_a_bundle(tmp_path):
    for unique_urls in (False, True):
        fixture = Fixture(tmp_path / str(unique_urls), ("123", "124", "125"),
            pin_transform=(lambda p: p["images"]["orig"].update(url=p["images"]["orig"]["url"] + "?pin=" + p["id"])) if unique_urls else None)
        result = fixture.run()
        assert result["state"] == "completed"
        assert result["metrics"]["new_byte_objects"] == 1 and result["metrics"]["existing_byte_bindings"] == 2
        assert len(fixture.media_calls) == (3 if unique_urls else 1)
        assert fixture.service.library(fixture.lake["library_id"]).verify(deep=True)["objects"] == 1


def test_soft_row_and_metadata_limits_cut_batches_without_losing_results(tmp_path, monkeypatch):
    monkeypatch.setattr(outbox, "MAX_ROWS", 16)
    monkeypatch.setattr(outbox, "MAX_METADATA", 12 * 1024)
    fixture = Distinct(tmp_path, count=7)
    assert fixture.run()["state"] == "completed"
    with fixture.state.db() as db:
        groups = db.execute("SELECT batch_id,count(*),sum(row_count),sum(metadata_bytes) FROM pinterest_outbox GROUP BY batch_id").fetchall()
    for _, count, rows, metadata in groups:
        assert count == 1 or (rows <= 16 and metadata <= 12 * 1024)
    assert sum(r[1] for r in groups) == 14


@pytest.mark.parametrize("point", ["pinterest_after_freeze", "pinterest_during_pack", "pinterest_after_commit", "pinterest_during_cleanup"])
def test_hard_process_death_recovers_saved_media_without_new_requests(tmp_path, point):
    fixture = Distinct(tmp_path / "lake", count=4)
    helper = tmp_path / "crash_worker.py"
    helper.write_text('''import os, sys
from pathlib import Path
from test_pinterest_batching import Distinct
from studio_lake.pinterest.service import Service
from studio_lake.updates.state import State
root, job, point = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
fixture=object.__new__(Distinct)
fixture.state=State(root/'controller')
fixture.service=Service(fixture.state)
fixture.job=fixture.service.job(job)
fixture.lake={'library_id':fixture.job['library_id']}
fixture.pin_calls,fixture.media_calls=[],[]
fixture.pin_transform=lambda p:p['images']['orig'].update(url=p['images']['orig']['url']+'?pin='+p['id'])
def event(value):
    with (root/'requests.log').open('a',encoding='utf-8') as output:
        output.write(value+'\\n')
        output.flush()
        os.fsync(output.fileno())
factory=fixture.factory
def counted_factory(*a,**kw):
    client=factory(*a,**kw)
    pin=client.pin
    def counted(identity):
        event('api:'+identity)
        return pin(identity)
    client.pin=counted
    return client
fixture.factory=counted_factory
get=fixture.get
def counted_get(url,**kw):
    result=get(url,**kw)
    event('image:'+url)
    if point!='resume' and len(fixture.media_calls)==4:
        os.environ['DANBOORU_STORE_FAIL_AT']=point
        os.environ['DANBOORU_STORE_HARD_EXIT']='1'
    return result
fixture.get=counted_get
execution=fixture.runner()
result=execution.run(job,time_slice=60)
assert result['state']=='completed',result
assert execution.resources.snapshot()['staging_reserved_bytes']==0
assert fixture.service.library(fixture.lake['library_id']).verify(deep=True)['objects']==4
''', encoding="utf-8")
    worker = Path(__file__).resolve().parents[1]
    env = {**os.environ, "PYTHONPATH": os.pathsep.join((str(worker / "src"), str(worker / "tests")))}
    command = [sys.executable, str(helper), str(tmp_path / "lake"), fixture.job["id"]]
    crashed = subprocess.run([*command, point], env=env, capture_output=True, timeout=60)
    assert crashed.returncode != 0
    recovered = subprocess.run([*command, "resume"], env=env, capture_output=True, timeout=60)
    assert recovered.returncode == 0, recovered.stderr.decode(errors="replace")
    events = (tmp_path / "lake" / "requests.log").read_text(encoding="utf-8").splitlines()
    assert len(events) == len(set(events)) == 8
    assert sum(s.startswith("api:") for s in events) == 4
