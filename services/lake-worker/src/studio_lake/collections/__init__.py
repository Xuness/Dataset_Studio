"""Versioned, bounded source collection control shared by CLI and Studio."""

CONTRACT_VERSION = 2
TERMINAL = {"completed", "completed_with_gaps", "cancelled"}
TASK_TERMINAL = {"done", "unavailable", "excluded", "needs_review", "cancelled"}
