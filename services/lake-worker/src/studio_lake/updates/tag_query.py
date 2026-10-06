"""Single-tag discovery streams with bounded, same-observation boolean filtering."""

import copy

from .archive import online
from .references import categories
from .sites import UpdateError


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


def initialize_cursor(lib, scope, cursor):
    streams = anchors(scope["query"], lib)
    lower = scope.get("start_id", 1)
    return {**cursor, "scope": "tag_query_at_request_time", "tag_anchors": streams,
            "tag_branch": 0, "next_id": lower, "upper": scope.get("end_id") if streams else lower,
            "metadata_complete": not streams, "metadata_records": 0, "matched_records": 0}


def page(runner, lib, job, site, check, cancelled, publication):
    scope, before = job["definition"]["range"], job["cursor"]
    cursor = copy.deepcopy(before)
    if cursor["metadata_complete"]:
        return
    anchor = cursor["tag_anchors"][cursor["tag_branch"]]
    lower, upper = cursor["next_id"], cursor["upper"]
    size = min(site.capabilities()["page_size"], job["definition"]["item_budget"] - cursor.get("slice_items", 0))
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
        if exhausted:
            cursor["tag_branch"] += 1
            cursor["metadata_complete"] = cursor["tag_branch"] == len(cursor["tag_anchors"])
            cursor["next_id"] = upper if cursor["metadata_complete"] else scope.get("start_id", 1)
        else:
            cursor["next_id"] = next_id
        cursor["pages"] += 1
        cursor["slice_pages"] += 1
        cursor["slice_items"] += len(rows)
        cursor["metadata_records"] += len(rows)
        cursor["matched_records"] += len(selected)
        cursor["replay_saved_response"] = False
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
