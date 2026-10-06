"""Single-tag discovery streams with bounded, same-observation boolean filtering."""

import copy
from datetime import datetime, timezone

from .archive import online
from .references import categories
from .sites import UpdateError
from . import query_cache
from ..util import now, read_json


def normalize(value, *, require_positive=True):
    if not isinstance(value, dict) or set(value) - {"all", "any", "none"}:
        raise UpdateError("INVALID_INPUT", "Tag query requires all, any and none lists")
    result = {}
    for name in ("all", "any", "none"):
        tags = value.get(name, [])
        if not isinstance(tags, list) or len(tags) > 64:
            raise UpdateError("INVALID_INPUT", "Each tag group accepts at most 64 literal tags")
        for tag in tags:
            if (not isinstance(tag, str) or not 1 <= len(tag) <= 256
                    or any(c.isspace() or ord(c) < 32 or c in ':*"\\{}' for c in tag)
                    or tag[0] in "-~(" or tag.lower() in {"and", "or", "not"}):
                raise UpdateError("INVALID_INPUT", "Use exact source tags, not metatags, wildcards or search operators")
        result[name] = sorted(set(tags))
    if sum(map(len, result.values())) > 128:
        raise UpdateError("INVALID_INPUT", "Tag query exceeds 128 tags")
    if require_positive and not (result["all"] or result["any"]):
        raise UpdateError("INVALID_INPUT", "Remote tag discovery requires at least one positive tag")
    return result


def tags_of(record, site):
    return set(record["tag_string" if site == "danbooru" else "tags"].split(" ")) - {""}


def matches(query, tags):
    return (set(query["all"]).issubset(tags)
            and (not query["any"] or not set(query["any"]).isdisjoint(tags))
            and set(query["none"]).isdisjoint(tags))


def anchors(query, lib=None):
    excluded = set(query["none"])
    if excluded.intersection(query["all"]) or (query["any"] and excluded.issuperset(query["any"])):
        return []
    if not query["all"]:
        return [tag for tag in query["any"] if tag not in excluded]
    counts = {}
    if lib is not None:
        # Local counts only estimate cost; they never establish remote coverage.
        with online(lib) as (db, _):
            for tag in query["all"]:
                row = next(db.execute("SELECT tag_id FROM tags WHERE tag=?", (tag,)), None)
                if row:
                    count = next(db.execute("SELECT doc FROM tag_vocabulary WHERE term=?", (f"t{row[0]:x}",)), None)
                    if count:
                        counts[tag] = count[0]
    return [min(query["all"], key=lambda tag: (counts.get(tag, float("inf")), tag))]


def initialize_cursor(runner, lib, job, site, scope, cursor):
    if scope.get("source") == "local":
        from .catalog import version
        from .runner import lease

        revision, seq, generation = version(lib, scope.get("version"))
        lease(lib, job["id"], seq)
        lower = scope.get("start_id", 1)
        upper = scope.get("end_id")
        if upper is None:
            with online(lib) as (db, _):
                row = next(db.execute("SELECT post_id+1 FROM post_versions WHERE valid_from<=? "
                                      "AND (valid_until IS NULL OR valid_until>?) ORDER BY post_id DESC LIMIT 1", (seq, seq)), None)
                upper = max(lower, row[0] if row else lower)
        return {**cursor, "scope": "local_metadata_at_version", "tag_anchors": [], "tag_branch": 0,
                "next_id": lower, "upper": upper, "metadata_complete": lower >= upper,
                "input_seq": seq, "input_generation": generation, "catalog_version": revision,
                "metadata_records": 0, "matched_records": 0, "metadata_reused_records": 0,
                "metadata_cache_pages": 0}
    streams = anchors(scope["query"], lib)
    lower = scope.get("start_id", 1)
    hours = query_cache.max_age(scope)
    context = site.query_context
    if scope["query"]["all"] and streams and hours:
        hits = [(tag, query_cache.find_page(runner.state, lib, context, tag, lower, hours)) for tag in scope["query"]["all"]]
        cached = [(tag, page) for tag, page in hits if page]
        if cached:
            streams = [max(cached, key=lambda item: (item[1]["through_id"], item[1]["observed_unix"]))[0]]
    upper = scope.get("end_id") if streams else lower
    observed = now()
    if streams and upper is None:
        cached = query_cache.cached_upper(runner.state, lib, context, hours)
        if cached and cached[0] >= lower:
            upper, at = cached
            observed = datetime.fromtimestamp(at, timezone.utc).isoformat()
    return {**cursor, "scope": "tag_query_at_request_time", "tag_anchors": streams,
            "tag_branch": 0, "next_id": lower,
            "metadata_complete": not streams, "metadata_records": 0, "matched_records": 0,
            "tag_context": context, "tag_generation": read_json(lib.cache / "ONLINE.json")["generation"],
            "tag_head_bound": scope.get("end_id") is None, "tag_head_observed_at": observed,
            "metadata_reused_records": 0, "metadata_cache_pages": 0, "upper": upper}


def validate_context(state, lib, site, cursor):
    if cursor.get("tag_context") and (cursor["tag_context"] != site.query_context
                                     or cursor["tag_context"] != state.access_context(site.name)):
        raise UpdateError("UPDATE_SCOPE_CHANGED", "Access conditions changed; create a new tag task instead of mixing scan coverage")
    if cursor.get("tag_generation") and read_json(lib.cache / "ONLINE.json")["generation"] != cursor["tag_generation"]:
        raise UpdateError("SOURCE_CHANGED", "Tag scan belongs to another online generation")


def advance(scope, cursor, next_id, returned, matched):
    if next_id >= cursor["upper"]:
        cursor["tag_branch"] += 1
        cursor["metadata_complete"] = cursor["tag_branch"] == len(cursor["tag_anchors"])
        cursor["next_id"] = cursor["upper"] if cursor["metadata_complete"] else scope.get("start_id", 1)
    else:
        cursor["next_id"] = next_id
    cursor["pages"] += 1
    cursor["slice_pages"] += 1
    cursor["slice_items"] += returned
    cursor["metadata_records"] += returned
    cursor["matched_records"] += matched
    cursor["replay_saved_response"] = False


def page(runner, lib, job, site, check, cancelled, publication):
    scope, before = job["definition"]["range"], job["cursor"]
    cursor = copy.deepcopy(before)
    if cursor["metadata_complete"]:
        return
    if scope.get("source") == "local":
        return local_page(runner, lib, job, site, check, publication)
    validate_context(runner.state, lib, site, cursor)
    anchor = cursor["tag_anchors"][cursor["tag_branch"]]
    lower, upper = cursor["next_id"], cursor["upper"]
    if lower >= upper:
        cursor.update(metadata_complete=True, tag_branch=len(cursor["tag_anchors"]))
        runner.save_cursor(lib, job, cursor)
        return
    size = min(site.capabilities()["page_size"], job["definition"]["item_budget"] - cursor.get("slice_items", 0))
    cached = query_cache.find_page(runner.state, lib, site.query_context, anchor, lower, query_cache.max_age(scope))
    if cached and not cursor.get("replay_saved_response"):
        records, next_id = query_cache.page_records(lib, cached, lower, upper, size)
        selected = {r["post_id"] for r in records if matches(scope["query"], tags_of(r["record"], site.name))}
        advance(scope, cursor, next_id, len(records), len(selected))
        cursor["metadata_reused_records"] = cursor.get("metadata_reused_records", 0) + len(records)
        cursor["metadata_cache_pages"] = cursor.get("metadata_cache_pages", 0) + 1
        check()
        query_cache.commit_reuse(runner.state, lib, job, records, selected, cursor, page=cached, publication=publication)
        return
    params = site.params(lower, upper, size, tag=anchor)
    response = runner.request_page(lib, job, site, params, cancelled)
    try:
        rows = site.parse(response)
        received = [r[1]["id"] for r in rows]
        if (len(rows) > size or received != sorted(received)
                or any(pid < lower or pid >= upper for pid in received)):
            raise UpdateError("UPDATE_PAGE_INVALID", "API ignored the tag stream's frozen ID bounds or order")
        if any(anchor not in tags_of(record, site.name) for _, record in rows):
            raise UpdateError("UPDATE_PAGE_INVALID", "API did not return the requested literal tag; use its canonical source name")
        selected = {record["id"] for _, record in rows if matches(scope["query"], tags_of(record, site.name))}
        next_id = received[-1] + 1 if received else upper
        exhausted = next_id >= upper
        query_page = dict(anchor=anchor, lower=lower, upper=upper, next_id=next_id,
                          exhausted=exhausted, branch=cursor["tag_branch"])
        advance(scope, cursor, next_id, len(rows), len(selected))
        types = categories(runner.state, lib, job, site, rows, set(received), cancelled)
        check()
        runner.state.progress(job["id"], phase="publishing_metadata", metadata_bytes_delta=len(response.body))
        runner.publish_page(lib, job, site, response, rows, selected, cursor,
                            metadata_ids=set(received), query_page=query_page, tag_types=types, publication=publication)
    except UpdateError as error:
        if error.code != "CANCELLED":
            runner.publish_page(lib, job, site, response, [], set(), before,
                                error=error, publication=publication)
        raise


def local_page(runner, lib, job, site, check, publication):
    from .catalog import read_page

    scope, cursor = job["definition"]["range"], copy.deepcopy(job["cursor"])
    size = min(200, job["definition"]["item_budget"] - cursor.get("slice_items", 0))
    result = read_page(lib, dict(query=scope["query"], start_id=scope.get("start_id", 1),
                                end_id=cursor["upper"], after=cursor["next_id"] - 1, limit=size,
                                version=cursor["catalog_version"], post_ids=scope.get("post_ids"), missing_media=scope.get("missing_media", False)),
                       retain=False, scan_limit=min(2048, job["definition"]["item_budget"] - cursor.get("slice_items", 0)))
    refs = [dict(post_id=r["post_id"], observation_id=r["observation_id"]) for r in result["items"]]
    records = query_cache.observation_records(lib, refs)
    cursor["metadata_complete"] = result["next_after"] is None
    cursor["next_id"] = result["next_after"] + 1 if result["next_after"] is not None else cursor["upper"]
    cursor["pages"] += 1
    cursor["slice_pages"] += 1
    cursor["slice_items"] += result["scanned"]
    cursor["metadata_records"] += result["scanned"]
    cursor["matched_records"] += len(records)
    cursor["metadata_reused_records"] += len(records)
    cursor["metadata_cache_pages"] += 1
    check()
    query_cache.commit_reuse(runner.state, lib, job, records, {r["post_id"] for r in records}, cursor, publication=publication)
