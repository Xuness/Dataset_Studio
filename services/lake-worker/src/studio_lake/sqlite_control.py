"""Small DB-API-compatible control connection on the patched SQLite runtime.

The archive/control SQL uses tuple-or-name rows and lazy transactions. Keep that
interface without using the older SQLite bundled with the host's Python.
"""

import apsw


class Row(tuple):
    def __new__(cls, values, names):
        row = super().__new__(cls, values)
        row.names = names
        return row

    def keys(self):
        return self.names

    def __getitem__(self, key):
        return super().__getitem__(self.names.index(key) if isinstance(key, str) else key)


class Cursor:
    def __init__(self, cursor, changed):
        self.cursor, self.rowcount = cursor, changed

    def __iter__(self):
        return iter(self.cursor)

    def fetchone(self):
        return self.cursor.fetchone()

    def fetchall(self):
        return self.cursor.fetchall()


class Connection:
    def __init__(self, path, timeout=30):
        if tuple(map(int, apsw.sqlitelibversion().split("."))) < (3, 51, 3):
            raise RuntimeError("Control writes require SQLite 3.51.3 or newer")
        self.raw = apsw.Connection(str(path))
        self.raw.set_busy_timeout(int(timeout * 1000))
        self.raw.set_row_trace(lambda cursor, row: Row(row, [v[0] for v in cursor.get_description()]))

    def _begin(self, sql):
        command = sql.lstrip().split(None, 1)[0].upper()
        if self.raw.get_autocommit() and command not in {
            "SELECT",
            "PRAGMA",
            "EXPLAIN",
            "BEGIN",
            "COMMIT",
            "ROLLBACK",
            "VACUUM",
        }:
            self.raw.execute("BEGIN IMMEDIATE")

    def execute(self, sql, args=()):
        self._begin(sql)
        cursor = self.raw.execute(sql, args)
        return Cursor(cursor, self.raw.changes())

    def executemany(self, sql, rows):
        self._begin(sql)
        cursor = self.raw.executemany(sql, rows)
        return Cursor(cursor, self.raw.changes())

    def executescript(self, sql):
        self._begin(sql)
        self.raw.execute(sql)

    def __enter__(self):
        return self

    def __exit__(self, kind, *_):
        if not self.raw.get_autocommit():
            self.raw.execute("ROLLBACK" if kind else "COMMIT")

    def close(self):
        self.raw.close()
