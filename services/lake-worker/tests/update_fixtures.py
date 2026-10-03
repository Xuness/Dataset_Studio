"""Build real pre-collection schemas before testing historical control upgrades."""


def remove_collection_schema(db):
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
    db.execute("DROP TABLE collection_quarantines")
    db.execute("DROP TABLE collection_batches")
    db.execute("ALTER TABLE collection_tasks DROP COLUMN download_generation")
    db.execute("ALTER TABLE collection_outbox DROP COLUMN outcome_state")
