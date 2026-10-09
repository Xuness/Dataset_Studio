"""Pinterest-owned bounded discovery. Every decision below is replayed only after archive acceptance."""

import json
import re

from . import normalize, topics
from ..canonical import canonical
from ..util import stable_id

MAX_PAGE_ITEMS = 250


def task(task_kind, subject, **payload):
    return dict(kind=task_kind, pin_id=subject, input=payload)


def metric(replay, name, value=1):
    replay.setdefault("metrics", {})[name] = replay.setdefault("metrics", {}).get(name, 0) + value


def admitted(db, job, kind, subject):
    return db.execute("SELECT 1 FROM pinterest_admitted WHERE job_id=? AND kind=? AND source_id=?", (job, kind, subject)).fetchone() is not None


def intent(spec, job_id, replay):
    result = []
    for seed in spec["seeds"]:
        subject, kind = seed["id"], seed["kind"]
        if kind == "pin":
            result.append(task("pin_detail", subject, pin_id=subject))
            if enabled(spec, "related_pins", 0):
                stream(replay, job_id, "related_pins", subject, seed, 1)
        elif kind in ("search_pins", "search_boards"):
            stream(replay, job_id, "search_page", subject, seed, scope="pins" if kind == "search_pins" else "boards")
        elif kind == "topic":
            stream(replay, job_id, "topic_page", subject, seed)
        elif subject.startswith("https:"):
            result.append(task(kind + "_resolve", subject, subject_id=subject, root=seed, depth=0))
        else:
            result.append(task("board_admit", subject, subject_id=subject, source_kind=kind, root=seed, depth=0))
    return result


def enabled(spec, entrypoint, depth):
    return entrypoint in spec["discovery"]["entrypoints"] and depth < spec["discovery"]["max_depth"]


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
        if enabled(spec, "board_more_ideas", entry["depth"]):
            stream(replay, job["id"], "board_more_ideas", subject, entry["root"], entry["depth"] + 1)
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
        metadata = json.loads(job["definition_json"])["metadata"]
        enrichment = metadata["detail_enrichment"]
        scheduled = 0
        if enrichment == "sample":
            with state.db() as db:
                scheduled = db.execute("SELECT count(*) FROM pinterest_tasks WHERE job_id=? AND json_extract(input_json,'$.scan_id')=? AND kind='pin_enrichment'", (job["id"], entry["scan_id"])).fetchone()[0]
        if enrichment == "all" or (enrichment == "sample" and scheduled < metadata.get("sample_size", 3)):
            replay["next_tasks"].append(task("pin_enrichment", subject, pin_id=subject, scan_id=entry["scan_id"],
                list_revision=parsed["content_revision"], purpose=enrichment))
        elif enrichment == "none":
            metric(replay, "unvalidated_list_manifests")
    metric(replay, entry["entrypoint"] + ":admitted_pins")
    spec = json.loads(job["definition_json"])
    if enabled(spec, "related_pins", entry.get("depth", 0)):
        stream(replay, job["id"], "related_pins", subject, entry["root"], entry.get("depth", 0) + 1)
    if entry.get("board_id") and enabled(spec, "pin_boards", entry.get("depth", 0)):
        replay["next_tasks"].append(task("board_admit", entry["board_id"], subject_id=entry["board_id"],
            source_kind="board", root=entry["root"], depth=entry.get("depth", 0) + 1))


def downloads(replay, parsed, scan_id=None, *, confirmed_detail=False):
    for item in parsed["entries"]:
        replay["next_tasks"].append(task("media_download", item["pin_id"], **item,
            context_id=parsed["context_id"], confirmed_detail=confirmed_detail, **({"scan_id": scan_id} if scan_id else {})))


def detail_expansion(state, job, entry, records, replay):
    spec = json.loads(job["definition_json"])
    root, depth = dict(kind="pin", id=entry["pin_id"]), 0
    if entry.get("scan_id"):
        with state.db() as db:
            row = db.execute("SELECT root_json,depth FROM pinterest_streams WHERE scan_id=?", (entry["scan_id"],)).fetchone()
        if row:
            root, depth = json.loads(row[0]), row[1]
    if not enabled(spec, "pin_boards", depth):
        return
    for entity in records.get("source_entities", []):
        if entity["kind"] == "board":
            replay["next_tasks"].append(task("board_admit", entity["source_id"], subject_id=entity["source_id"],
                source_kind="board", root=root, depth=depth + 1))


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
    topic, topic_options, decoded = None, None, None
    if kind == "topic_page" and entry.get("cursor") is None and response.status == 200:
        try:
            decoded, topic, topic_options = topics.document(response.body, subject)
        except (ValueError, KeyError, TypeError, UnicodeError):
            pass
    records, payload, error = normalize.capture_facts(response, replay["receipt_id"], subject, parsed_payload=decoded)
    visibility = {k: response.context[k] for k in ("mode", "language", "session_id", "anonymous_cookie_policy", "source_country", "source_language", "source_locale") if response.context.get(k) is not None}
    with state.db() as db:
        saved = db.execute("SELECT parameters_json FROM pinterest_streams WHERE scan_id=? AND job_id=?", (scan, job["id"])).fetchone()
    old_visibility = json.loads(saved[0]).get("visibility", {}) if saved else {}
    if any(k in visibility and visibility[k] != v for k, v in old_visibility.items()):
        error = "discovery_visibility_changed"
        replay["checkpoint"] = dict(scan_id=scan, page_key=stable_id("pinterest-page-v1", entry.get("cursor")),
            cursor=entry.get("cursor"), state="needs_review", reason=error, members=0, visibility=old_visibility)
    visibility = {**old_visibility, **visibility}
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
    if cursor and (previous or cursor == entry.get("cursor")):
        reason, exhausted = "repeated_cursor", False
    capture = records["captures"][0]
    snapshot = stable_id("pinterest-discovery-v1", capture["capture_id"], scan)
    members, seen = [], set()
    spec = json.loads(job["definition_json"])
    entity = None
    if kind in ("board_page", "board_sections", "board_more_ideas", "section_page"):
        parent = "section" if kind == "section_page" else "board"
        entity = normalize.entity_facts(records, dict(id=subject), parent, response)
    if topic:
        entity = normalize.entity_facts(records, topic, "topic", response)
        boards = topic.get("related_boards", [])
        if not isinstance(boards, list) or len(boards) > MAX_PAGE_ITEMS:
            replay.update(state="needs_review", reason="topic_related_boards_shape_invalid")
            records["captures"][0]["source_error"] = replay["reason"]
            return records
        for ordinal, board in enumerate(boards):
            if not isinstance(board, dict) or not isinstance(board.get("id"), str) or not re.fullmatch(r"[1-9][0-9]{0,19}", board["id"]):
                continue
            normalize.entity_facts(records, board, "board", response)
            if enabled(spec, "topic_boards", entry["depth"]):
                replay["next_tasks"].append(task("board_admit", board["id"], subject_id=board["id"], source_kind="board",
                    root=entry["root"], depth=entry["depth"] + 1, discovery_ordinal=ordinal, capture_id=capture["capture_id"]))
    ignored = 0
    for ordinal, item in enumerate(items):
        if not isinstance(item, dict) or not isinstance(item.get("id"), str) or not re.fullmatch(r"[1-9][0-9]{0,19}", item["id"]):
            ignored += 1
            continue
        identity = item["id"]
        if kind == "search_page" and entry["parameters"]["scope"] == "boards":
            if item.get("type") != "board":
                ignored += 1
                continue
            normalize.entity_facts(records, item, "board", response)
            replay["next_tasks"].append(task("board_admit", identity, subject_id=identity, source_kind="board",
                root=entry["root"], depth=entry["depth"], discovery_ordinal=ordinal, capture_id=capture["capture_id"]))
            replay.setdefault("seen", []).append(dict(entrypoint="search_boards", kind="boards", value=identity))
            continue
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
        parsed = normalize.pin_facts(records, item, response, kind="list", field_set=topic_options.get("field_set_key") if topic_options else None)
        if entity:
            role = {"section_page": "section_member", "board_page": "board_member",
                    "board_more_ideas": "recommended_for_board", "topic_page": "topic_result"}[kind]
            normalize.relation(records, identity, entity, role)
        if kind == "section_page" and entry.get("parameters", {}).get("board_id"):
            board = normalize.entity_facts(records, dict(id=entry["parameters"]["board_id"]), "board", response)
            normalize.relation(records, identity, board, "board_member")
        board = item.get("board")
        replay["next_tasks"].append(task("pin_admit", identity, pin_id=identity, scan_id=scan,
            entrypoint=kind, parsed=parsed, root=entry["root"], depth=entry["depth"],
            board_id=board.get("id") if isinstance(board, dict) and isinstance(board.get("id"), str) else None))
        replay.setdefault("seen", []).append(dict(entrypoint=kind, kind="pins", value=identity))
        signature = item.get("image_signature")
        if isinstance(signature, str) and signature:
            replay["seen"].append(dict(entrypoint=kind, kind="signatures", value=signature))
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
        state="needs_review" if reason else "exhausted" if exhausted else "active",
        reason=reason or ("empty_result_reason_unknown" if not items else None), members=len(members), visibility=visibility)
    metric(replay, kind + ":pages")
    metric(replay, kind + ":members", len(members))
    metric(replay, kind + ":ignored_items", ignored)
    if not items:
        metric(replay, kind + ":empty_pages")
    if reason:
        replay.update(state="needs_review", reason=reason)
    elif not exhausted:
        parameters = {**entry["parameters"], **({"topic_options": topic_options} if topic_options else {})}
        replay["next_tasks"].append(task(kind, subject, **{**entry, "cursor": cursor, "parameters": parameters}))
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
