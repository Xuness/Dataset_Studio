"""System-protected secrets: Windows DPAPI or Linux Secret Service, no plaintext fallback."""

import ctypes
from ctypes import wintypes
import json
import os
import sys

from .sites import UpdateError

REQUEST_PREFIX = "dpapi:" if os.name == "nt" else "protected:"


def protect(value, encrypt):
    if sys.platform == "linux":
        from .linux_credentials import protect as linux_protect
        return linux_protect(value, encrypt)
    if os.name != "nt":
        raise UpdateError("UPDATE_CREDENTIAL_REQUIRED", "A platform secret provider is required")

    class Blob(ctypes.Structure):
        _fields_ = [("size", wintypes.DWORD), ("data", ctypes.POINTER(ctypes.c_ubyte))]

    buf = (ctypes.c_ubyte * len(value)).from_buffer_copy(value)
    source, target = Blob(len(value), buf), Blob()
    crypt = ctypes.WinDLL("crypt32", use_last_error=True)
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.LocalFree.argtypes, kernel.LocalFree.restype = [ctypes.c_void_p], ctypes.c_void_p
    function = crypt.CryptProtectData if encrypt else crypt.CryptUnprotectData
    function.argtypes = [
        ctypes.POINTER(Blob),
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_void_p,
        wintypes.DWORD,
        ctypes.POINTER(Blob),
    ]
    function.restype = wintypes.BOOL
    try:
        if not function(ctypes.byref(source), None, None, None, None, 1, ctypes.byref(target)):
            raise UpdateError("UPDATE_CREDENTIAL_REQUIRED", "Windows could not unlock these credentials")
        return ctypes.string_at(target.data, target.size)
    finally:
        ctypes.memset(buf, 0, len(value))
        if target.data:
            ctypes.memset(target.data, 0, target.size)
            kernel.LocalFree(target.data)


def validate(site, value):
    allowed = {"danbooru": {"login", "api_key"}, "gelbooru": {"user_id", "api_key"}, "yandere": set()}
    if site not in allowed or not isinstance(value, dict) or set(value) != allowed[site]:
        raise UpdateError("INVALID_INPUT", "Credential fields do not match the site")
    for key, text in value.items():
        if (
            not isinstance(text, str)
            or not text.strip()
            or len(text) > 8192
            or any(ord(c) < 32 for c in text)
        ):
            raise UpdateError("INVALID_INPUT", "Credential is empty or invalid")
        if key == "user_id" and (not text.isdigit() or int(text) <= 0):
            raise UpdateError("INVALID_INPUT", "Gelbooru user_id must be a positive integer")
    return value


def encode(site, value):
    return protect(json.dumps(validate(site, value), separators=(",", ":")).encode(), True)


def decode(blob):
    return json.loads(protect(blob, False))
