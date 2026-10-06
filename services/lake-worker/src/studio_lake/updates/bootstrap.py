"""Idempotent empty Booru archives and online-v2 projections, without a producer index."""

from contextlib import closing
from pathlib import Path
import uuid

from .. import __version__
from ..collections import requests as ledger
from ..collections.model import fields, identity
from ..config import Config
from ..library import JOURNAL_SQL
from ..online_schema import VERSION, schema_sql, set_state, settings
from ..online_storage import connect
from ..sqlite_control import Connection
from ..util import FileLock, IntegrityError, atomic_json, contained, failpoint, now, read_json, sync_file
from .sites import Site, UpdateError


def checked_json(path, value):
    if path.exists():
        if read_json(path) != value:
            raise IntegrityError("Lake initialization conflicts with an existing identity or location")
    else:
        atomic_json(path, value)


def initialize(config, site, library_id):
    """A durable intent owns both directories before creating any lake files."""
    config.root.mkdir(parents=True, exist_ok=True)
    config.cache.mkdir(parents=True, exist_ok=True)
    with FileLock(config.root / ".writer.lock"), FileLock(config.cache / ".initialize.lock"):
        intent_path = config.root / "initialization.json"
        expected = dict(library_id=library_id, site=site, media_root=str(config.root),
                        index_root=str(config.cache), format_version=1, online_version=VERSION)
        if intent_path.exists():
            intent = read_json(intent_path)
            if any(intent.get(key) != value for key, value in expected.items()):
                raise IntegrityError("Another initialization owns these lake directories")
        else:
            if any(p.name != ".writer.lock" for p in config.root.iterdir()) or any(
                p.name != ".initialize.lock" for p in config.cache.iterdir()
            ):
                raise UpdateError("UPDATE_TARGET_NOT_EMPTY", "New lake directories must be empty; register an existing lake instead")
            intent = {**expected, "generation": "online-" + uuid.uuid4().hex, "created_at": now()}
            atomic_json(intent_path, intent)
        info = dict(format_version=1, library_id=library_id, site=site,
                    created_at=intent["created_at"], created_by=__version__,
                    image_format="uncompressed-pax-tar",
                    source_policy="preserve-all-fields-and-original-api-response-bodies")
        marker = config.root / "library.json"
        journal = config.root / "journal.sqlite"
        if marker.exists():
            existing = read_json(marker)
            if any(existing.get(key) != info[key] for key in ("library_id", "format_version", "site")):
                raise IntegrityError("Existing archive differs from this initialization")
            if not journal.is_file():
                raise IntegrityError("Archive journal is missing; refusing to reset an existing lake")
        else:
            with closing(Connection(journal)) as db, db:
                tables = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")}
                if tables and tables != {"commits", "progress", "settings", "releases"}:
                    raise IntegrityError("Unrecognized journal in the initialization directory")
                db.executescript(JOURNAL_SQL)
            sync_file(journal)
            atomic_json(marker, info)
        for name in ("segments", "staging", "plans", "releases", "publish", "source_manifests"):
            (config.root / name).mkdir(exist_ok=True)
        failpoint("after_empty_lake_archive")

        pointer = dict(schema_version=VERSION, library_id=library_id, site=site,
                       generation=intent["generation"], file=f"online/{intent['generation']}.sqlite")
        database = contained(config.cache, pointer["file"])
        database.parent.mkdir(exist_ok=True)
        if (config.cache / "ONLINE.json").exists() and not database.is_file():
            raise IntegrityError("Published online database is missing; use archive recovery")
        with closing(connect(database)) as db:
            if not next(db.execute("SELECT 1 FROM sqlite_master WHERE name='online_state'"), None):
                if next(db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"), None):
                    raise IntegrityError("Unrecognized online database in the initialization directory")
                with db:
                    db.execute(schema_sql())
                    set_state(db, schema_version=VERSION, library_id=library_id, site=site,
                              generation=intent["generation"], archive_seq=0, served_seq=0,
                              analysis_seq=0, base_seq=0, min_seq=0, media_root=str(config.root),
                              source_index=str(config.cache), source_generation="archive",
                              source="canonical-archive-v1", object_count=0, observation_count=0, asset_count=0)
                    db.execute("INSERT INTO publications VALUES(0,'empty-baseline',?,0,0,0)", (intent["created_at"],))
            state = settings(db)
            if any(state.get(key) != str(pointer[key]) for key in ("schema_version", "library_id", "site", "generation")):
                raise IntegrityError("Online initialization identity was replaced")
        failpoint("after_empty_lake_online")
        checked_json(config.cache / "cache_owner.json", dict(library_id=library_id, root=str(config.root)))
        checked_json(config.cache / "ONLINE.json", pointer)
        checked_json(config.root / "online-index.json", dict(schema_version=VERSION, library_id=library_id, index_root=str(config.cache)))
        failpoint("after_empty_lake_pointers")


def create(state, args):
    fields(args, ("request_key", "site", "media_root", "index_root"))
    identity(args["request_key"])
    if args["site"] not in Site.URLS:
        raise UpdateError("INVALID_INPUT", "Unsupported Booru source")
    for key in ("media_root", "index_root"):
        if not isinstance(args[key], str) or not args[key] or len(args[key]) > 32768 or not Path(args[key]).is_absolute():
            raise UpdateError("INVALID_INPUT", "Lake directories must be absolute paths")
    try:
        config = Config(Path(args["media_root"]), Path(args["index_root"]))
    except ValueError as error:
        raise UpdateError("INVALID_INPUT", str(error)) from None
    args = {**args, "media_root": str(config.root), "index_root": str(config.cache)}
    # A stable UUID survives a lost response or an interrupted filesystem initialization.
    with state.db() as db:
        db.execute("BEGIN IMMEDIATE")
        request, _ = ledger.begin(db, "booru_lake_create", args["request_key"], args, str(uuid.uuid4()))
        library_id = request["subject_id"]
        if request["state"] == "succeeded":
            return state.lake(library_id)
    with FileLock(state.root / "lake-initializations" / (args["request_key"] + ".lock")):
        initialize(config, args["site"], library_id)
        lake = state.register(dict(library_id=library_id, site=args["site"],
                                   media_root=str(config.root), index_root=str(config.cache)))
        with state.db() as db:
            ledger.succeed(db, args["request_key"], library_id)
    return lake
