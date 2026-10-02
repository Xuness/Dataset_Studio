"""Build real pre-collection schemas before testing historical control upgrades."""


def remove_collection_schema(db):
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
