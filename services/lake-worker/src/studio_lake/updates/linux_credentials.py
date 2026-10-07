"""AES-GCM envelopes; key material is held by the desktop Secret Service.

Format and namespace match studio-llm/src/credentials/linux.rs. The on-disk
marker contains only a random key ID. Missing/locked keys never get replaced.
"""
import base64
import fcntl
from functools import lru_cache
import os
from pathlib import Path
import re
import subprocess
import tempfile
import uuid

from .sites import UpdateError

MAGIC = b"DSC1"
HEADER = 36


def unavailable():
    return UpdateError("UPDATE_CREDENTIAL_REQUIRED", "Unlock the desktop keyring and install libsecret-tools")


def keyring(identity, secret=None):
    args = (["store", "--label=Dataset Studio credential key"] if secret is not None else ["lookup"])
    try:
        result = subprocess.run(
            ["secret-tool", *args, "application", "com.xuness.datasetstudio", "key", identity],
            input=base64.b64encode(secret) if secret is not None else b"",
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=30, check=True,
        )
        return result.stdout
    except (OSError, subprocess.SubprocessError):
        raise unavailable() from None


@lru_cache(maxsize=8)
def key(identity):
    try:
        value = base64.b64decode(keyring(identity).strip(), validate=True)
        if len(value) != 32:
            raise ValueError()
        return value
    except ValueError:
        raise unavailable() from None


@lru_cache(maxsize=1)
def active_id():
    base = Path(os.environ.get("XDG_DATA_HOME", ""))
    if not base.is_absolute():
        base = Path.home() / ".local/share"
    root = base / "dataset-studio"
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    root.chmod(0o700)
    fd = os.open(root / "credential-key.lock", os.O_CREAT | os.O_RDWR, 0o600)
    with os.fdopen(fd, "r+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        marker = root / "credential-key"
        if marker.exists():
            identity = marker.read_text(encoding="ascii").strip()
            if not re.fullmatch("[0-9a-f]{32}", identity):
                raise unavailable()
            return identity
        identity = uuid.uuid4().hex
        material = os.urandom(32)
        keyring(identity, material)
        if key(identity) != material:
            raise unavailable()
        with tempfile.NamedTemporaryFile(dir=root, delete=False) as file:
            temporary = Path(file.name)
            file.write(identity.encode("ascii"))
            file.flush()
            os.fsync(file.fileno())
        try:
            temporary.replace(marker)
        finally:
            temporary.unlink(missing_ok=True)
        return identity


def protect(value, encrypt):
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    from cryptography.exceptions import InvalidTag

    try:
        if encrypt:
            identity = active_id()
            header = MAGIC + identity.encode("ascii")
            nonce = os.urandom(12)
            return header + nonce + AESGCM(key(identity)).encrypt(nonce, value, header)
        if len(value) < HEADER + 12 + 16 or value[:4] != MAGIC:
            raise ValueError()
        identity = value[4:HEADER].decode("ascii")
        if not re.fullmatch("[0-9a-f]{32}", identity):
            raise ValueError()
        return AESGCM(key(identity)).decrypt(value[HEADER:HEADER+12], value[HEADER+12:], value[:HEADER])
    except (OSError, ValueError, UnicodeError, InvalidTag):
        raise unavailable() from None
