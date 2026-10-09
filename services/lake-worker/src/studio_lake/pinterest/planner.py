"""Pinterest-owned bounded discovery. Every decision below is replayed only after archive acceptance."""

import json
import re

from . import normalize
from ..canonical import canonical
from ..util import stable_id

MAX_PAGE_ITEMS = 250


def task(task_kind, subject, **payload):
    return dict(kind=task_kind, pin_id=subject, input=payload)


def metric(replay, name, value=1):
    replay.setdefault("metrics", {})[name] = replay.setdefault("metrics", {}).get(name, 0) + value


def admitted(db, job, kind, subject):
    return db.execute("SELECT 1 FROM pinterest_admitted WHERE job_id=? AND kind=? AND source_id=?", (job, kind, subject)).fetchone() is not None


def intent(spec):
    result = []
    for seed in spec["seeds"]:
        subject, kind = seed["id"], seed["kind"]
        if kind == "pin":
            result.append(task("pin_detail", subject, pin_id=subject))
        elif subject.startswith("https:"):
            result.append(task(kind + "_resolve", subject, subject_id=subject, root=seed, depth=0))
        else:
            result.append(task("board_admit", subject, subject_id=subject, source_kind=kind, root=seed, depth=0))
    return result


def stream(replay, job_id, kind, subject, root, depth=0, **parameters):
    scan = stable_id("pinterest-scan-v1", job_id, kind, subject, root, parameters)
    entry = dict(scan_id=scan, entrypoint=kind, subject_id=subject, root=root, depth=depth, parameters=parameters, cursor=None)
    replay.setdefault("streams", []).append(entry)
    replay["next_tasks"].append(task(kind, subject, **entry))


def admission(state, job, entry, replay):
    spec = json.loads(job["definition_json"])
    subject, kind = entry["subject_id"], entry["source_kind"]
    with state.db() as db:
        if admitted(db, job["id"], kind, subject):
            return
    replay.setdefault("admissions", []).append(dict(kind=kind, source_id=subject))
    if kind == "board":
        stream(replay, job["id"], "board_page", subject, entry["root"], entry["depth"])
        if spec.get("discovery", {}).get("include_sections", True):
            stream(replay, job["id"], "board_sections", subject, entry["root"], entry["depth"])
    else:
        stream(replay, job["id"], "section_page", subject, entry["root"], entry["depth"], board_id=entry.get("board_id"))


def admit_pin(state, job, entry, replay):
    subject = entry["pin_id"]
    with state.db() as db:
        if admitted(db, job["id"], "pin", subject):
            return
        force = db.execute("SELECT force_detail FROM pinterest_streams WHERE scan_id=?", (entry["scan_id"],)).fetchone()
    replay.setdefault("admissions", []).append(dict(kind="pin", source_id=subject))
    parsed = entry["parsed"]
    if parsed["state"] != "ready" or (force and force[0]):
        replay["next_tasks"].append(task("pin_detail", subject, pin_id=subject, scan_id=entry["scan_id"]))
    else:
        downloads(replay, parsed, entry["scan_id"])
        if json.loads(job["definition_json"])["metadata"]["detail_enrichment"] == "none":
            metric(replay, "unvalidated_list_manifests")
    metric(replay, entry["entrypoint"] + ":admitted_pins")


def downloads(replay, parsed, scan_id=None):
    for item in parsed["entries"]:
        replay["next_tasks"].append(task("media_download", item["pin_id"], **item,
            context_id=parsed["context_id"], **({"scan_id": scan_id} if scan_id else {})))


def resolved(response, job, entry, replay):
    records, payload, error = normalize.capture_facts(response, replay["receipt_id"], entry["subject_id"])
    envelope = payload.get("resource_response") if isinstance(payload, dict) else None
    data = envelope.get("data") if isinstance(envelope, dict) else None
    if not error and (not isinstance(data, dict) or not isinstance(data.get("id"), str)
                      or not re.fullmatch(r"[1-9][0-9]{0,19}", data["id"])):
        error = "source_identity_or_shape_invalid"
    if error:
        replay.update(state="needs_review", reason=error)
        records["captures"][0]["source_error"] = error
        return records
    kind = "board" if entry["root"]["kind"] == "board" else "section"
    normalize.entity_facts(records, data, kind, response)
    board = data.get("board")
    board_id = board.get("id") if isinstance(board, dict) else None
    if board_id:
        normalize.entity_facts(records, board, "board", response)
    replay["next_tasks"].append(task("board_admit", data["id"], subject_id=data["id"], source_kind=kind,
        root=entry["root"], depth=entry["depth"], board_id=board_id))
    return records


def pagination(payload):
    resource = payload.get("resource")
    options = resource.get("options") if isinstance(resource, dict) else None
    cursor = options.get("bookmarks") if isinstance(options, dict) else None
    if (not isinstance(cursor, list) or len(cursor) > 16 or len(canonical(cursor).encode()) > 16 * 1024
            or any(not isinstance(v, str) or not v for v in cursor)):
        return None, False, "pagination_signal_missing_or_invalid"
    if not cursor or cursor == ["-end-"]:
        return None, True, None
    if "-end-" in cursor:
        return None, False, "pagination_signal_conflict"
    return cursor, False, None


def page(state, response, job, entry, replay):
    kind, subject, scan = entry["entrypoint"], entry["subject_id"], entry["scan_id"]
    records, payload, error = normalize.capture_facts(response, replay["receipt_id"], subject)
    envelope = payload.get("resource_response") if isinstance(payload, dict) else None
    data = envelope.get("data") if isinstance(envelope, dict) else None
    items = data.get("results") if isinstance(data, dict) else data
    if not error and (not isinstance(items, list) or len(items) > MAX_PAGE_ITEMS):
        error = "discovery_response_shape_invalid"
    if error:
        records["captures"][0]["source_error"] = error
        replay.update(state="needs_review", reason=error)
        return records
    cursor, exhausted, reason = pagination(payload)
    key = stable_id("pinterest-page-v1", entry.get("cursor"))
    with state.db() as db:
        previous = db.execute("SELECT 1 FROM pinterest_stream_pages WHERE scan_id=? AND page_key=?",
                              (scan, stable_id("pinterest-page-v1", cursor))).fetchone() if cursor else None
        scheduled = db.execute("SELECT count(*) FROM pinterest_tasks WHERE job_id=? AND json_extract(input_json,'$.scan_id')=? AND kind='pin_enrichment'", (job["id"], scan)).fetchone()[0]
    if cursor and (previous or cursor == entry.get("cursor")):
        reason, exhausted = "repeated_cursor", False
    capture = records["captures"][0]
    snapshot = stable_id("pinterest-discovery-v1", capture["capture_id"], scan)
    members, seen = [], set()
    spec = json.loads(job["definition_json"])
    enrichment = spec["metadata"]["detail_enrichment"]
    parent = "section" if kind == "section_page" else "board"
    entity = normalize.entity_facts(records, dict(id=subject), parent, response)
    ignored = 0
    for ordinal, item in enumerate(items):
        if not isinstance(item, dict) or not isinstance(item.get("id"), str) or not re.fullmatch(r"[1-9][0-9]{0,19}", item["id"]):
            ignored += 1
            continue
        identity = item["id"]
        if kind == "board_sections":
            if item.get("type") not in (None, "board_section", "section"):
                ignored += 1
                continue
            normalize.entity_facts(records, item, "section", response)
            replay["next_tasks"].append(task("board_admit", identity, subject_id=identity, source_kind="section",
                root=entry["root"], depth=entry["depth"], board_id=subject))
            continue
        if item.get("type") != "pin":
            ignored += 1
            continue
        members.append(dict(snapshot_id=snapshot, ordinal=ordinal, pin_id=identity))
        if identity in seen:
            continue
        seen.add(identity)
        parsed = normalize.pin_facts(records, item, response, kind="list")
        normalize.relation(records, identity, entity, "section_member" if kind == "section_page" else "board_member")
        if kind == "section_page" and entry.get("parameters", {}).get("board_id"):
            board = normalize.entity_facts(records, dict(id=entry["parameters"]["board_id"]), "board", response)
            normalize.relation(records, identity, board, "board_member")
        replay["next_tasks"].append(task("pin_admit", identity, pin_id=identity, scan_id=scan,
            entrypoint=kind, parsed=parsed))
        sample = scheduled < spec["metadata"].get("sample_size", 3)
        if parsed["state"] == "ready" and (enrichment == "all" or (enrichment == "sample" and sample)):
            scheduled += 1
            replay["next_tasks"].append(task("pin_enrichment", identity, pin_id=identity,
                scan_id=scan, list_revision=parsed["content_revision"], purpose="sample" if enrichment == "sample" else "all"))
    if items and not members and kind != "board_sections" and ignored == len(items):
        # Known placeholders are recorded without inventing Pin identities; unknown shapes stay reviewable.
        if any(not isinstance(v, dict) or v.get("type") not in ("story", "ad", "separator") for v in items):
            reason, exhausted = "unrecognized_discovery_items", False
    records["discovery_snapshots"] = [dict(snapshot_id=snapshot, capture_id=capture["capture_id"],
        entrypoint=kind, scan_id=scan, root_json=canonical(entry["root"]), context_id=capture["context_id"],
        observed_at=response.observed_at, page_key=key + ":" + replay["receipt_id"],
        next_cursor_json=canonical(cursor) if cursor else None, complete=reason is None, exhausted=exhausted)]
    records["discovery_members"] = members
    replay["checkpoint"] = dict(scan_id=scan, page_key=key, cursor=cursor,
        state="needs_review" if reason else "exhausted" if exhausted else "active", reason=reason, members=len(members))
    metric(replay, kind + ":pages")
    metric(replay, kind + ":members", len(members))
    metric(replay, kind + ":ignored_items", ignored)
    if not items:
        metric(replay, kind + ":empty_pages")
    if reason:
        replay.update(state="needs_review", reason=reason)
    elif not exhausted:
        replay["next_tasks"].append(task(kind, subject, **{**entry, "cursor": cursor}))
    return records


def comparison(replay, entry, parsed):
    if not entry.get("list_revision"):
        return
    differs = parsed.get("content_revision") != entry["list_revision"]
    replay["sample"] = dict(scan_id=entry["scan_id"], differs=differs)
    metric(replay, "sampled_manifests")
    if differs:
        metric(replay, "manifest_discrepancies")
        replay["reason"] = "list_detail_media_difference"
