"""Release only obsolete automatic publication tags; explicit pins always survive.

The engine holds the writer gate and excludes in-flight readers for an apply.
Unknown recent files are retained by Lance's normal transaction safety window.
"""
from datetime import datetime, timedelta, timezone
import re


def cleanup(vectors, recipe, published_version, protected_versions, keep_last=8,
            min_age_hours=168, apply=False):
    if not 1 <= keep_last <= 1000 or not 0 <= min_age_hours <= 87600:
        raise ValueError("invalid retention policy")
    table = vectors.table(recipe)
    versions = table.list_versions()
    tags = table.tags.list()
    known = {int(v["version"]) for v in versions}
    protected = {int(v) for v in protected_versions} | {int(published_version), int(table.version)}
    if not protected.issubset(known):
        raise ValueError("a referenced feature version is missing; cleanup refused")
    tag_versions = {int(value["version"]) for value in tags.values()}
    if not (protected - {int(table.version)}).issubset(tag_versions):
        raise ValueError("a referenced version has no retention pin; cleanup refused")
    recent = set(sorted(known, reverse=True)[:keep_last])
    cutoff = datetime.now(timezone.utc) - timedelta(hours=min_age_hours)
    old = set()
    for row in versions:
        stamp = row["timestamp"]
        if stamp.tzinfo is None:
            # LanceDB 0.40 uses datetime.fromtimestamp(), i.e. local time.
            stamp = stamp.astimezone(timezone.utc)
        if stamp < cutoff:
            old.add(int(row["version"]))
    released = [name for name, value in tags.items()
                if re.fullmatch(r"published_[0-9]+", name)
                and int(value["version"]) in old - recent - protected]
    explicit = {int(value["version"]) for name, value in tags.items() if name not in released}
    candidates = old - recent - protected - explicit
    result = {"recipe_id": recipe["id"], "published_version": int(published_version),
              "keep_last": keep_last, "min_age_hours": min_age_hours, "applied": bool(apply),
              "versions_before": len(known), "candidate_versions": len(candidates),
              "released_publication_tags": len(released), "protected_versions": len(protected | explicit),
              "removed_versions": 0, "bytes_removed": 0}
    if apply:
        for name in released:
            table.tags.delete(name)
        stats = table.to_lance().cleanup_old_versions(
            older_than=timedelta(seconds=max(.001, min_age_hours * 3600)),
            retain_versions=keep_last, delete_unverified=False,
            error_if_tagged_old_versions=False,
        )
        result["removed_versions"] = int(stats.old_versions)
        result["bytes_removed"] = int(stats.bytes_removed)
        table.checkout_latest()
        for version in protected | recent:
            table.checkout(version)
        table.checkout_latest()
    return result
