"""Failure meanings shared by task execution and the job coordinator."""

from contextlib import contextmanager
import errno
import zipfile
import zlib

from ..updates.sites import UpdateError
from ..util import IntegrityError


class InvalidReceipt(IntegrityError):
    """An owned, uncommitted result cannot become canonical facts."""


def exception_code(error):
    if isinstance(error, UpdateError):
        return "COLLECTION_LIMIT" if error.code == "UPDATE_RESOURCE_LIMIT" else error.code
    if isinstance(error, OSError):
        if error.errno in {errno.ENOSPC, getattr(errno, "EDQUOT", errno.ENOSPC)} or getattr(error, "winerror", None) in {39, 112}:
            return "UPDATE_SPACE"
        return "COLLECTION_IO"
    return "COLLECTION_INTEGRITY"


def task_state(code, attempts):
    if code == "CANCELLED":
        return "queued"
    if code in {"COLLECTION_CREDENTIAL_REQUIRED", "COLLECTION_SCOPE_CHANGED"}:
        return "waiting_credentials"
    if code == "UPDATE_SPACE":
        return "waiting_resources"
    if code in {"COLLECTION_NOT_ACCESSIBLE", "COLLECTION_SOURCE_CHANGED"}:
        return "unavailable"
    if code == "COLLECTION_REMOTE_UNAVAILABLE" and attempts < 8:
        return "retry_wait"
    return "needs_review"


@contextmanager
def decoding():
    """Only parsing/encoding runs here; storage errors keep their own meaning."""
    try:
        yield
    except OSError as error:
        if error.errno is not None or getattr(error, "winerror", None) is not None:
            raise
        raise UpdateError("COLLECTION_MEDIA_INVALID", "Media decoding failed") from error
    except (zipfile.BadZipFile, zlib.error, IntegrityError, ValueError, SyntaxError) as error:
        raise UpdateError("COLLECTION_MEDIA_INVALID", "Media decoding failed") from error
