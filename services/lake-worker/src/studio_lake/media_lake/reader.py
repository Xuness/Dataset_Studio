"""Bounded read model for the collector CLI; Studio reads the same format natively."""

import base64
import json
from pathlib import Path

import apsw

from .records import one
from .schema import canonical
from ..library import read_object
from ..raw_codec import decode
from ..util import IntegrityError, contained, digest, read_json


class Reader:
    def __init__(self, media, index, *, version=None):
        self.media_root = Path(media)
        self.pointer = read_json(Path(index) / "ONLINE.json")
        if self.pointer["schema_version"] != 3 or self.pointer["library_id"] != read_json(self.media_root / "library.json")["library_id"]:
            raise IntegrityError("Read identity or format mismatch")
        self.db = apsw.Connection(str(contained(Path(index), self.pointer["file"])), flags=apsw.SQLITE_OPEN_READONLY)
        self.db.set_busy_timeout(500)
        self.db.execute("BEGIN")
        state = dict(self.db.execute("SELECT key,value FROM online_state"))
        prefix = "online-v3:" + self.pointer["generation"] + ":"
        if version is not None and (not version.startswith(prefix) or not version[len(prefix):].isdigit()):
            self.close()
            raise IntegrityError("Read version belongs to another generation")
        self.seq = int(version[len(prefix):]) if version is not None else int(state["served_seq"])
        if not int(state["min_seq"]) <= self.seq <= int(state["served_seq"]):
            self.close()
            raise IntegrityError("Read version expired or not yet published")
        self.version = prefix + str(self.seq)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def close(self):
        self.db.close()

    def cursor(self, kind, selector, last):
        return base64.urlsafe_b64encode(canonical(dict(version=self.version, kind=kind, selector=selector, last=last)).encode()).decode()

    def position(self, cursor, kind, selector, default):
        if cursor is None:
            return default
        if not isinstance(cursor, str) or len(cursor) > 4096:
            raise IntegrityError("Invalid read cursor")
        try:
            value = json.loads(base64.urlsafe_b64decode(cursor))
        except (ValueError, UnicodeError):
            raise IntegrityError("Invalid read cursor") from None
        if any(value.get(k) != v for k, v in dict(version=self.version, kind=kind, selector=selector).items()):
            raise IntegrityError("Read cursor version or selector changed")
        return value["last"]

    def work(self, identity):
        result = one(self.db, """SELECT * FROM work_versions WHERE work_id=? AND valid_from<=?
            AND (valid_until IS NULL OR valid_until>?)""", (identity, self.seq, self.seq))
        if result is None:
            raise KeyError(identity)
        detail = one(self.db, "SELECT * FROM work_observations WHERE observation_id=? AND commit_seq<=?", (result["observation_id"], self.seq))
        manifest = one(self.db, "SELECT * FROM media_manifests WHERE manifest_id=? AND commit_seq<=?", (result["manifest_id"], self.seq))
        return dict(work_id=identity, observation=detail, manifest=manifest, manifest_state=result["manifest_state"], version=self.version)

    def media(self, work_id, *, cursor=None, limit=50, manifest_id=None, recipe_id=None):
        if not 1 <= limit <= 200:
            raise IntegrityError("Page size must be 1–200")
        if manifest_id is None:
            manifest = self.work(work_id)["manifest"]
            manifest_id = manifest["manifest_id"] if manifest else None
        if manifest_id is None:
            return dict(work_id=work_id, manifest_id=None, items=[], next_cursor=None, version=self.version)
        manifest = one(self.db, "SELECT * FROM media_manifests WHERE manifest_id=? AND work_id=? AND commit_seq<=?", (manifest_id, work_id, self.seq))
        if manifest is None:
            raise KeyError(manifest_id)
        selector = dict(work_id=work_id, manifest_id=manifest_id, recipe_id=recipe_id)
        after = self.position(cursor, "media", selector, -1)
        rows = list(self.db.execute("SELECT media_id FROM media_entries WHERE manifest_id=? AND ordinal>? AND commit_seq<=? ORDER BY ordinal LIMIT ?",
                                   (manifest_id, after, self.seq, limit + 1)))
        items = []
        for (identity,) in rows[:limit]:
            row = one(self.db, "SELECT * FROM media_entries WHERE media_id=?", (identity,))
            bindings = []
            for (asset_id,) in self.db.execute("""SELECT v.asset_id FROM media_asset_versions v WHERE media_id=?
                AND valid_from<=? AND (valid_until IS NULL OR valid_until>?)
                AND (? IS NULL OR recipe_id=? OR representation IN ('original','poster'))
                ORDER BY representation,recipe_id LIMIT 9""", (identity, self.seq, self.seq, recipe_id, recipe_id)):
                bindings.append(one(self.db, "SELECT a.*,o.media_category,o.content_type,o.stored_width,o.stored_height FROM assets a JOIN objects o USING(sha256) WHERE a.asset_id=?", (asset_id,)))
            if len(bindings) > 8:
                raise IntegrityError("Select one recipe to bound media bindings")
            items.append({**row, "bindings": bindings})
        return dict(work_id=work_id, manifest_id=manifest_id, items=items, next_cursor=self.cursor("media", selector, items[-1]["ordinal"]) if len(rows) > limit else None, version=self.version)

    def objects(self, *, cursor=None, limit=50):
        if not 1 <= limit <= 200:
            raise IntegrityError("Page size must be 1–200")
        after = self.position(cursor, "objects", {}, "")
        rows = list(self.db.execute("SELECT sha256,length,stored_ext,stored_width,stored_height FROM objects WHERE first_seq<=? AND media_category='image' AND sha256>? ORDER BY sha256 LIMIT ?", (self.seq, after, limit + 1)))
        items = [dict(zip(("sha256", "bytes", "extension", "width", "height"), r)) for r in rows[:limit]]
        return dict(items=items, next_cursor=self.cursor("objects", {}, items[-1]["sha256"]) if len(rows) > limit else None, version=self.version)

    def raw(self, capture_id):
        row = one(self.db, "SELECT * FROM captures WHERE capture_id=? AND commit_seq<=?", (capture_id, self.seq))
        if row is None:
            raise KeyError(capture_id)
        status = "too_large" if row["raw_bytes"] > 131072 else "available"
        value = decode(row["raw_zlib"], row["raw_bytes"], row["raw_sha256"], maximum_bytes=131072) if status == "available" else None
        return dict(capture_id=capture_id, format=row["raw_format"], bytes=row["raw_bytes"], status=status, json=value, version=self.version)

    def blob(self, sha, *, maximum_bytes=128 * 1024**2):
        row = one(self.db, "SELECT * FROM objects WHERE sha256=? AND first_seq<=?", (sha, self.seq))
        if row is None:
            raise KeyError(sha)
        if row["length"] > maximum_bytes:
            raise IntegrityError("Object exceeds read budget")
        body = read_object(self.media_root, row["pack_path"], row["offset"], row["length"])
        if digest(body) != sha:
            raise IntegrityError("Object content hash mismatch")
        return body
