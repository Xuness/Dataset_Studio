"""Provider failures must never replace keys or fall back to plaintext."""
import base64
import os
import subprocess
import sys
from pathlib import Path

import pytest

if sys.platform != "linux":
    pytest.skip("Linux Secret Service adapter", allow_module_level=True)

from studio_lake.updates import linux_credentials as vault
from studio_lake.updates.sites import UpdateError


@pytest.fixture
def isolated_vault(tmp_path, monkeypatch):
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    vault.active_id.cache_clear()
    vault.key.cache_clear()
    saved = {}

    def keyring(identity, secret=None):
        if secret is not None:
            assert identity not in saved, "A missing key must never be replaced"
            saved[identity] = secret
        if identity not in saved:
            raise vault.unavailable()
        return base64.b64encode(saved[identity])

    monkeypatch.setattr(vault, "keyring", keyring)
    yield saved
    vault.active_id.cache_clear()
    vault.key.cache_clear()


def test_restart_nonce_authentication_and_marker_loss(isolated_vault, tmp_path):
    first = vault.protect(b"fixture-cookie", True)
    second = vault.protect(b"fixture-cookie", True)
    assert first != second and b"fixture-cookie" not in first
    vault.active_id.cache_clear()
    vault.key.cache_clear()
    assert vault.protect(first, False) == b"fixture-cookie"
    for position in [4, 36, len(first) - 1]:
        broken = bytearray(first)
        broken[position] ^= 1
        with pytest.raises(UpdateError):
            vault.protect(bytes(broken), False)
    (tmp_path / "dataset-studio/credential-key").unlink()
    vault.active_id.cache_clear()
    newer = vault.protect(b"new-account", True)
    assert len(isolated_vault) == 2
    assert vault.protect(first, False) == b"fixture-cookie"
    assert vault.protect(newer, False) == b"new-account"


def test_missing_key_never_rotates_or_overwrites(isolated_vault, tmp_path):
    value = vault.protect(b"saved", True)
    marker = tmp_path / "dataset-studio/credential-key"
    identity = marker.read_bytes()
    isolated_vault.clear()
    vault.key.cache_clear()
    for encrypt in [True, False]:
        with pytest.raises(UpdateError):
            vault.protect(value, encrypt)
    assert marker.read_bytes() == identity and not isolated_vault


def test_native_keyring_survives_a_new_process():
    # Run inside the test session's private D-Bus/keyring (documented CI entry).
    secret = b"linux-restart-fixture"
    value = vault.protect(secret, True)
    child = subprocess.run(
        [sys.executable, "-c", "import sys; from studio_lake.updates.credentials import protect; "
         "sys.stdout.buffer.write(protect(sys.stdin.buffer.read(), False))"],
        input=value, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=35,
        env={**os.environ, "PYTHONPATH": str(Path(__file__).resolve().parents[1] / "src")},
    )
    assert child.returncode == 0, "Keyring could not be reopened in a new process"
    assert child.stdout == secret
