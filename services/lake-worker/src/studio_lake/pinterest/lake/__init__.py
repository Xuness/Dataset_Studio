"""Pinterest canonical archive 3 and serving index 4."""

ARCHIVE_VERSION = 3
ONLINE_VERSION = 4
SCHEMA_SET = "pinterest-media-v1"
FEATURES = ("pinterest-pins-v1", "raw-captures-v1", "receipt-replay-v1", "sha256-objects-v1")
# Bundling changes archive envelopes, not the online fact schema or fixed versions.
BUNDLE_FEATURE = "receipt-bundles-v1"
ARCHIVE_FEATURES = (*FEATURES, BUNDLE_FEATURE)


def supported_archive_features(value):
    return value in (list(FEATURES), list(ARCHIVE_FEATURES))
