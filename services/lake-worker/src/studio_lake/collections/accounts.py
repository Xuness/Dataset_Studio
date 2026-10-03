"""Local Pixiv identities and bounded cookie imports, never exposed to media hosts."""

import json
import re
import uuid
from dataclasses import dataclass, field

from . import model, requests as ledger
from ..collectors.pixiv.normalize import visibility
from ..media_lake.schema import canonical, utc
from ..updates.credentials import protect
from ..updates.sites import UpdateError


def cookies(value):
    if not isinstance(value, list) or not 1 <= len(value) <= 128 or len(canonical(value).encode()) > 64 * 1024:
        model.invalid("Import 1–128 cookies within 64 KiB")
    normalized, seen = [], set()
    for item in value:
        model.fields(item, ("name", "value", "domain", "path", "secure", "http_only", "expires_unix"))
        if not isinstance(item["name"], str) or not re.fullmatch(r"[!#$%&'*+.^_`|~0-9A-Za-z-]{1,256}", item["name"]):
            model.invalid("Invalid cookie name")
        if not isinstance(item["value"], str) or not 1 <= len(item["value"]) <= 8192 or any(ord(c) < 32 or ord(c) >= 127 or c in ';\r\n' for c in item["value"]):
            model.invalid("Invalid cookie value")
        model.choice(item["domain"], (".pixiv.net", "pixiv.net", "www.pixiv.net", "accounts.pixiv.net"))
        if not isinstance(item["path"], str) or not item["path"].startswith("/") or len(item["path"]) > 1024 or any(ord(c) < 32 for c in item["path"]):
            model.invalid("Invalid cookie path")
        if item["secure"] is not True:
            model.invalid("Cookies must be restricted to HTTPS")
        model.boolean(item["http_only"])
        if item["expires_unix"] is not None:
            model.integer(item["expires_unix"])
        key = (item["name"], item["domain"], item["path"])
        if key in seen:
            model.invalid("Duplicate cookie identity")
        seen.add(key)
        normalized.append(dict(item))
    return sorted(normalized, key=lambda c: (c["domain"], c["path"], c["name"]))


@dataclass(frozen=True)
class VerifiedSession:
    context: dict
    user_id: str
    cookies: list = field(repr=False)


def verify_session(client, values, viewer):
    context = client.probe()
    policy = json.loads(context["policy_json"])
    if policy.get("login") != "authenticated":
        raise UpdateError("COLLECTION_CREDENTIAL_REQUIRED", "Pixiv did not confirm an authenticated user")
    identity = model.source_id(policy.get("user_id"))
    updated = cookies(client.cookie_snapshot()) if hasattr(client, "cookie_snapshot") else values
    context = visibility(viewer, login="authenticated", user_id=identity, observed_at=context["observed_at"])
    return VerifiedSession(context, identity, updated)


class Accounts:
    def __init__(self, state):
        self.state = state

    def row(self, identity, db=None):
        model.identity(identity)
        if db is None:
            with self.state.db() as db:
                return self.row(identity, db)
        row = db.execute("SELECT * FROM collection_accounts WHERE id=?", (identity,)).fetchone()
        if row is None:
            raise UpdateError("NOT_FOUND", "Collection account does not exist")
        return dict(row)

    def public(self, identity):
        row = self.row(identity)
        probe = json.loads(row["last_probe_json"]) if row["last_probe_json"] else None
        return dict(id=row["id"], site="pixiv", label=row["label"], mode=row["mode"], revision=row["revision"],
                    state=row["state"], credential_set=row["secret_blob"] is not None,
                    bound_user_id=row["bound_user_id"], last_probe_at=probe["observed_at"] if probe else None)

    def save(self, args):
        model.fields(args, ("request_key", "expected_revision", "account_id", "label", "mode"), ("cookies",))
        identity = model.identity(args["account_id"])
        mode = model.choice(args["mode"], ("anonymous", "session"))
        if not isinstance(args["label"], str) or not 1 <= len(args["label"]) <= 128:
            model.invalid("Account label must be 1–128 characters")
        values = cookies(args.get("cookies")) if mode == "session" else []
        if mode == "anonymous" and args.get("cookies"):
            model.invalid("Public sessions cannot contain cookies")
        args = {**args, "cookies": values}
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            old_request, _ = ledger.begin(db, "account_save", args["request_key"], args, identity, secret=mode == "session")
            if old_request["state"] == "succeeded":
                return self.public(identity)
            old = db.execute("SELECT * FROM collection_accounts WHERE id=?", (identity,)).fetchone()
            revision = old["revision"] if old else None
            if args["expected_revision"] != revision:
                raise UpdateError("REVISION_CONFLICT", "Account changed; reload before importing")
            if old and old["mode"] != mode:
                raise UpdateError("COLLECTION_SCOPE_CHANGED", "Use a new local identity when changing authentication mode")
            at = utc()
            viewer = old["viewer_key"] if old else str(uuid.uuid4())
            context = visibility(viewer, observed_at=at) if mode == "anonymous" else None
            secret = protect(canonical(values).encode(), True) if values else None
            db.execute("""INSERT INTO collection_accounts VALUES(?,'pixiv',?,?,?,NULL,?,1,?,?,?,?)
                ON CONFLICT(id) DO UPDATE SET label=excluded.label,secret_blob=excluded.secret_blob,
                revision=collection_accounts.revision+1,state=excluded.state,last_probe_json=excluded.last_probe_json,updated_at=excluded.updated_at""",
                       (identity, args["label"], mode, viewer, secret, "valid" if mode == "anonymous" else "unverified",
                        canonical(context) if context else None, at, at))
            ledger.succeed(db, args["request_key"], identity)
        return self.public(identity)

    def clear(self, identity, args):
        model.fields(args, ("request_key", "expected_revision"))
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            old, _ = ledger.begin(db, "account_clear", args["request_key"], dict(id=identity, **args), identity)
            if old["state"] != "succeeded":
                row = self.row(identity, db)
                if row["revision"] != args["expected_revision"]:
                    raise UpdateError("REVISION_CONFLICT", "Account revision changed")
                db.execute("UPDATE collection_accounts SET secret_blob=NULL,state='cleared',revision=revision+1,updated_at=? WHERE id=?", (utc(), identity))
                ledger.succeed(db, args["request_key"], identity)
        return self.public(identity)

    def authenticate(self, args, *, client=None):
        """Probe candidate credentials before atomically replacing the saved account."""
        model.fields(args, ("request_key", "expected_revision", "account_id", "label", "mode"), ("cookies",))
        identity = model.identity(args["account_id"])
        model.choice(args["mode"], ("session",))
        if not isinstance(args["label"], str) or not 1 <= len(args["label"]) <= 128:
            model.invalid("Account label must be 1–128 characters")
        values = cookies(args.get("cookies"))
        args = {**args, "cookies": values}

        def current(db):
            row = db.execute("SELECT * FROM collection_accounts WHERE id=?", (identity,)).fetchone()
            if args["expected_revision"] != (row["revision"] if row else None):
                raise UpdateError("REVISION_CONFLICT", "Account changed; restart the login assistant")
            if row and row["mode"] != "session":
                raise UpdateError("COLLECTION_SCOPE_CHANGED", "Use a new identity for a login session")
            return dict(row) if row else None

        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request, _ = ledger.begin(db, "account_authenticate", args["request_key"], args, identity, secret=True)
            if request["state"] == "succeeded":
                return dict(account=self.public(identity), visibility=json.loads(request["result_json"]))
            old = current(db)
        viewer = old["viewer_key"] if old else str(uuid.uuid4())
        candidate = dict(id=identity, mode="session", viewer_key=viewer)
        from ..collectors.pixiv.http import Client

        owned = client is None
        client = client or Client(self.state.root, candidate, values)
        try:
            verified = verify_session(client, values, viewer)
        finally:
            if owned:
                client.close()
        context, observed_user, values = verified.context, verified.user_id, verified.cookies
        report = dict(context_id=context["context_id"], observed_at=context["observed_at"],
                      **{k: v for k, v in json.loads(context["policy_json"]).items() if k != "user_id"}, coverage_verified=context["verified"])
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request, _ = ledger.begin(db, "account_authenticate", args["request_key"], args, identity, secret=True)
            if request["state"] == "succeeded":
                return dict(account=self.public(identity), visibility=json.loads(request["result_json"]))
            latest = current(db)
            if latest and latest["bound_user_id"] and latest["bound_user_id"] != observed_user:
                raise UpdateError("COLLECTION_SCOPE_CHANGED", "The browser identifies another Pixiv account; create a new account reference")
            at = utc()
            db.execute("""INSERT INTO collection_accounts VALUES(?,'pixiv',?,'session',?,?,?,1,'valid',?,?,?)
                ON CONFLICT(id) DO UPDATE SET label=excluded.label,secret_blob=excluded.secret_blob,
                bound_user_id=excluded.bound_user_id,revision=collection_accounts.revision+1,state='valid',
                last_probe_json=excluded.last_probe_json,updated_at=excluded.updated_at""",
                       (identity, args["label"], viewer, observed_user, protect(canonical(values).encode(), True),
                        canonical(context), at, at))
            ledger.succeed(db, args["request_key"], identity, report)
        return dict(account=self.public(identity), visibility=report)

    def session(self, identity, *, require_valid=True):
        row = self.row(identity)
        if require_valid and row["state"] != "valid":
            raise UpdateError("COLLECTION_CREDENTIAL_REQUIRED", "Import and verify this account before collection")
        values = json.loads(protect(row["secret_blob"], False)) if row["secret_blob"] is not None else []
        if row["mode"] == "session" and not values:
            raise UpdateError("COLLECTION_CREDENTIAL_REQUIRED", "The selected session has no credentials")
        context = json.loads(row["last_probe_json"]) if row["last_probe_json"] else visibility(row["viewer_key"], login="unknown")
        return row, values, context

    def expire(self, identity, revision):
        with self.state.db() as db:
            db.execute("UPDATE collection_accounts SET state='expired',revision=revision+1,updated_at=? WHERE id=? AND revision=?", (utc(), identity, revision))

    def renew(self, identity, revision, values):
        values = cookies(values)
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            row = self.row(identity, db)
            if row["revision"] != revision or row["state"] != "valid" or row["mode"] != "session":
                return False
            old = json.loads(protect(row["secret_blob"], False))
            if canonical(old) == canonical(values):
                return True
            db.execute("UPDATE collection_accounts SET secret_blob=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?",
                       (protect(canonical(values).encode(), True), utc(), identity, revision))
        return True

    def probe(self, identity, args, *, client=None):
        model.fields(args, ("request_key", "expected_revision"))
        row = self.row(identity)
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            request, _ = ledger.begin(db, "account_probe", args["request_key"], dict(id=identity, **args), identity)
            if request["state"] == "succeeded":
                return dict(account=self.public(identity), visibility=json.loads(request["result_json"]))
            if row["revision"] != args["expected_revision"]:
                raise UpdateError("REVISION_CONFLICT", "Account revision changed")
        row, values, _ = self.session(identity, require_valid=False)
        if row["mode"] == "anonymous":
            context = visibility(row["viewer_key"])
        else:
            from ..collectors.pixiv.http import Client

            owned = client is None
            client = client or Client(self.state.root, row, values)
            try:
                verified = verify_session(client, values, row["viewer_key"])
                context, values = verified.context, verified.cookies
            finally:
                if owned:
                    client.close()
        observed_user = json.loads(context["policy_json"])["user_id"]
        with self.state.db() as db:
            db.execute("BEGIN IMMEDIATE")
            current = self.row(identity, db)
            if current["revision"] != args["expected_revision"]:
                raise UpdateError("REVISION_CONFLICT", "A newer credential import superseded this probe")
            if current["bound_user_id"] and current["bound_user_id"] != observed_user:
                raise UpdateError("COLLECTION_SCOPE_CHANGED", "These cookies identify a different Pixiv account; create a new account reference")
            secret = protect(canonical(values).encode(), True) if current["mode"] == "session" else None
            db.execute("UPDATE collection_accounts SET bound_user_id=?,last_probe_json=?,secret_blob=?,state='valid',revision=revision+1,updated_at=? WHERE id=?",
                       (observed_user, canonical(context), secret, utc(), identity))
            report = dict(context_id=context["context_id"], observed_at=context["observed_at"],
                          **{k: v for k, v in json.loads(context["policy_json"]).items() if k != "user_id"}, coverage_verified=context["verified"])
            ledger.succeed(db, args["request_key"], identity, report)
        return dict(account=self.public(identity), visibility=report)
