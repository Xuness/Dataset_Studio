"""Build real pre-collection schemas before testing historical control upgrades."""


def remove_collection_schema(db):
    remove_pinterest_schema(db)
    db.execute("DROP INDEX IF EXISTS update_job_created")
    names = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'collection_%'")}
    for (name,) in db.execute("SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE 'collection_%'").fetchall():
        db.execute('DROP TRIGGER "' + name + '"')
    while names:
        parents = {r[2] for table in names for r in db.execute('PRAGMA foreign_key_list("' + table + '")') if r[2] in names}
        leaves = names - parents
        assert leaves, "Fixture dependency graph must be acyclic"
        for name in sorted(leaves):
            db.execute('DROP TABLE "' + name + '"')
        names -= leaves


def remove_collection_recovery(db):
    """Recreate the v10 column shape before testing a real older-version upgrade."""
    remove_pinterest_schema(db)
    db.execute("DROP TABLE collection_quarantines")
    db.execute("DROP TABLE collection_batches")
    db.execute("ALTER TABLE collection_tasks DROP COLUMN download_generation")
    db.execute("ALTER TABLE collection_outbox DROP COLUMN outcome_state")


def remove_pinterest_schema(db):
    """Historical control fixtures predate the separately versioned Pinterest tables."""
    for name in ("pinterest_schedule_runs", "pinterest_schedules", "pinterest_reuse", "pinterest_seen", "pinterest_stream_pins", "pinterest_stream_pages", "pinterest_streams", "pinterest_admitted_counts", "pinterest_admitted", "pinterest_metrics",
                 "pinterest_downloads", "pinterest_applied", "pinterest_counts", "pinterest_tasks", "pinterest_jobs", "pinterest_requests", "pinterest_lakes"):
        db.execute('DROP TABLE IF EXISTS "' + name + '"')


def remove_pinterest_pagination(db):
    """Recreate the real v19 discovery shape before testing its upgrade."""
    db.execute("DROP TABLE pinterest_stream_pins")
    db.execute("ALTER TABLE pinterest_streams DROP COLUMN unique_pins")
    db.execute("ALTER TABLE pinterest_streams DROP COLUMN reported_total")
