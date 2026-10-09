"""Immutable captures, Pin observations and manifests from the same response evidence."""

import json

from . import NORMALIZER, media_manifest
from ..canonical import canonical
from ..util import digest, stable_id


def response_facts(response, receipt_id, pin_id):
    capture_id = stable_id("pinterest-capture-v1", receipt_id)
    context_id = stable_id("pinterest-context-v1", response.context)
    records = dict(visibility_contexts=[dict(context_id=context_id, policy_json=canonical(response.context),
                                             observed_at=response.context["created_at"])])
    payload, error, reason = None, None, None
    try:
        payload = json.loads(response.body)
        canonical(payload)
        raw_format = "json"
    except (ValueError, UnicodeError):
        payload = None
        raw_format = "text"
    data = payload.get("resource_response") if isinstance(payload, dict) else None
    if response.status != 200:
        error = reason = "pin_http_" + str(response.status)
    elif not isinstance(data, dict) or data.get("status") not in (None, "success") or data.get("error"):
        error = reason = "pin_response_invalid"
    elif not isinstance(data.get("data"), dict) or data["data"].get("id") != pin_id:
        error = reason = "pin_identity_or_shape_invalid"
    records["captures"] = [dict(capture_id=capture_id, request_receipt_id=receipt_id, endpoint=response.endpoint,
        subject_id=pin_id, request_json=canonical(response.parameters), context_id=context_id,
        observed_at=response.observed_at, http_status=response.status, source_error=error, adapter_version=NORMALIZER,
        raw_format=raw_format, raw_bytes=len(response.body), raw_sha256=digest(response.body), raw_body=response.body)]
    if error:
        return records, dict(state="needs_review", reason=reason, entries=[], capture_id=capture_id)
    pin = data["data"]
    parsed = media_manifest.parse(pin)
    observation = stable_id("pinterest-observation-v1", capture_id, NORMALIZER, pin_id)
    manifest = stable_id("pinterest-manifest-v1", observation)
    records["pins"] = [dict(pin_id=pin_id)]
    records["pin_observations"] = [dict(observation_id=observation, pin_id=pin_id, capture_id=capture_id,
        observed_at=response.observed_at, normalizer_version=NORMALIZER, observation_kind="detail",
        field_set=response.parameters["options"]["field_set_key"], title=pin.get("title") if isinstance(pin.get("title"), str) else None,
        description=pin.get("description") if isinstance(pin.get("description"), str) else None,
        image_signature=pin.get("image_signature") if isinstance(pin.get("image_signature"), str) else None,
        fields_json=canonical(pin), present_fields_json=canonical(sorted(pin)),
        issues_json=canonical([parsed["reason"]] if parsed["reason"] else []))]
    records["media_manifests"] = [dict(manifest_id=manifest, pin_id=pin_id, capture_id=capture_id,
        observation_id=observation, context_id=context_id, observed_at=response.observed_at, normalizer_version=NORMALIZER,
        kind=parsed["kind"], content_revision=parsed["content_revision"], expected_count=1 if parsed["complete"] else None,
        item_count=len(parsed["entries"]), complete=parsed["complete"], reason=parsed["reason"])]
    records["media_entries"] = [dict(**entry, media_id=stable_id("pinterest-media-v1", manifest, entry["slot_key"]),
                                    manifest_id=manifest, pin_id=pin_id) for entry in parsed["entries"]]
    entities, observations, relations = [], [], []
    for field, kind, role in (("board", "board", "saved_to"), ("pinner", "account", "pinner"),
                               ("origin_pinner", "account", "origin_pinner"), ("section", "section", "section")):
        entity = pin.get(field)
        if not isinstance(entity, dict) or not isinstance(entity.get("id"), str):
            continue
        identity = stable_id("pinterest-entity-v1", kind, entity["id"])
        if not any(v["entity_id"] == identity for v in entities):
            entities.append(dict(entity_id=identity, kind=kind, source_id=entity["id"]))
            observations.append(dict(observation_id=stable_id(identity, capture_id), entity_id=identity,
                capture_id=capture_id, observed_at=response.observed_at, fields_json=canonical(entity)))
        relations.append(dict(relation_id=stable_id(pin_id, identity, role, capture_id), pin_id=pin_id,
                              entity_id=identity, role=role, capture_id=capture_id))
    records.update(source_entities=entities, entity_observations=observations, source_relations=relations)
    return records, {**parsed, "entries": records["media_entries"], "capture_id": capture_id, "context_id": context_id}
