"""Current tags follow posts; observation history stays queryable without repeated materialization."""

from .util import IntegrityError, failpoint

VERSION = 1


def tag_names(column):
    # Match the existing literal-space tokenization; order and duplicates have no tag-set meaning.
    return f"list_sort(list_filter(list_distinct(string_split(coalesce({column},''),' ')), t -> t<>''))"


def version(con):
    if not con.execute("SELECT 1 FROM duckdb_tables() WHERE table_name='tag_projection_meta'").fetchone():
        return None
    return con.execute("SELECT version,tags_enabled FROM tag_projection_meta").fetchone()


def require_projection(con, enabled):
    state = version(con)
    if state is None:
        raise IntegrityError("SSD 标签投影需要升级，请先运行 upgrade-tag-projection；主库无需重建")
    if state[0] != VERSION:
        raise IntegrityError("SSD 标签投影版本不兼容")
    if state[1] != enabled:
        raise IntegrityError("改变 build_tags 后需要 rebuild-index，不能留下不完整的标签查询")


def create_history_view(con):
    boundary, enabled = con.execute("SELECT legacy_max_row,tags_enabled FROM tag_projection_meta").fetchone()
    # These values are immutable for the generation. Constant predicates filter
    # observations before lateral tag expansion; scalar metadata subqueries can
    # otherwise force DuckDB to expand all old observations before filtering.
    con.execute(f"""
        CREATE OR REPLACE VIEW tag_postings AS
        SELECT row_id,tag_id FROM archived_tag_postings
        UNION ALL
        SELECT o.row_id,t.tag_id FROM (
            SELECT row_id,tag_string FROM observations WHERE row_id>{int(boundary)} AND {bool(enabled)}
        ) o, unnest({tag_names("o.tag_string")}) AS n(tag) JOIN tags t ON t.tag=n.tag
    """)


def refresh_history_view(con):
    definition = con.execute("SELECT sql FROM duckdb_views() WHERE view_name='tag_postings'").fetchone()
    if definition and "tag_projection_meta" in definition[0]:
        create_history_view(con)


def initialize_projection(con, enabled, emit=None):
    """Called in one DuckDB transaction, either on an empty index or an isolated generation."""
    if version(con) is not None:
        require_projection(con, enabled)
        return
    con.execute("DROP INDEX IF EXISTS postings_tag")
    legacy = con.execute("SELECT 1 FROM duckdb_tables() WHERE table_name='tag_postings'").fetchone()
    if legacy:
        con.execute("ALTER TABLE tag_postings RENAME TO archived_tag_postings")
    else:
        con.execute("CREATE TABLE archived_tag_postings(row_id BIGINT,tag_id BIGINT)")
    boundary = con.execute("SELECT coalesce(max(row_id),0) FROM observations").fetchone()[0]
    count = con.execute("SELECT count(*) FROM archived_tag_postings").fetchone()[0]
    con.execute(
        "CREATE TABLE tag_projection_meta(version INTEGER PRIMARY KEY,tags_enabled BOOLEAN NOT NULL,"
        "legacy_max_row BIGINT NOT NULL,historical_count BIGINT NOT NULL)"
    )
    con.execute("INSERT INTO tag_projection_meta VALUES (?,?,?,?)", [VERSION, enabled, boundary, count])
    con.execute("CREATE TABLE current_tag_state(post_id BIGINT PRIMARY KEY,tag_fingerprint VARCHAR NOT NULL)")
    con.execute("CREATE TABLE current_tag_postings(post_id BIGINT,tag_id BIGINT)")
    # Existing relations remain immutable. Future history is derived from complete observations.
    create_history_view(con)
    if enabled:
        low, high = con.execute("SELECT min(post_id),max(post_id) FROM current_posts").fetchone()
        if low is not None:
            for start in range(low, high + 1, 250000):
                con.execute(
                    f"""
                    CREATE OR REPLACE TEMP TABLE current_tag_input AS
                    SELECT c.post_id,{tag_names("o.tag_string")} AS tag_names
                    FROM current_posts c JOIN observations o ON o.row_id=c.row_id
                    WHERE c.post_id>=? AND c.post_id<?
                """,
                    [start, start + 250000],
                )
                _insert_current(con)
                if emit:
                    emit("tag_projection_bootstrap", through_post_id=min(start + 249999, high))
    failpoint("after_tag_projection_bootstrap")


def _insert_current(con):
    con.execute(
        "INSERT INTO current_tag_postings SELECT p.post_id,t.tag_id "
        "FROM current_tag_input p,unnest(p.tag_names) AS n(tag) JOIN tags t ON t.tag=n.tag "
        "ORDER BY p.post_id,t.tag_id"
    )
    con.execute(
        "INSERT OR REPLACE INTO current_tag_state "
        "SELECT post_id,sha256(to_json(tag_names)) FROM current_tag_input"
    )


def update_current(con):
    """Use the actual winner, including asset-only batches and lower-priority observations."""
    con.execute(f"""
        CREATE OR REPLACE TEMP TABLE current_tag_input AS
        SELECT c.post_id,{tag_names("o.tag_string")} AS tag_names
        FROM current_posts c JOIN observations o ON o.row_id=c.row_id JOIN affected a ON a.post_id=c.post_id
    """)
    affected = con.execute("SELECT count(*) FROM current_tag_input").fetchone()[0]
    con.execute("""
        DELETE FROM current_tag_input USING current_tag_state s
        WHERE current_tag_input.post_id=s.post_id
          AND sha256(to_json(current_tag_input.tag_names))=s.tag_fingerprint
    """)
    changed, tags = con.execute(
        "SELECT count(*),coalesce(sum(len(tag_names)),0) FROM current_tag_input"
    ).fetchone()
    if changed:
        con.execute(
            "DELETE FROM current_tag_postings WHERE post_id IN (SELECT post_id FROM current_tag_input)"
        )
        failpoint("after_current_tags_deleted")
        _insert_current(con)
    return {"affected_posts": affected, "changed_posts": changed, "tags_written": tags}


def verify_current(con, enabled, emit=None):
    """Compare every current relation with its winning observation, without reading image objects."""
    require_projection(con, enabled)
    count = con.execute("SELECT count(*) FROM current_tag_state").fetchone()[0]
    postings = con.execute("SELECT count(*) FROM current_tag_postings").fetchone()[0]
    if not enabled:
        if count or postings:
            raise IntegrityError("禁用标签时当前投影不应包含记录")
        return {"posts": 0, "tag_postings": 0}
    expected = con.execute("SELECT count(*) FROM current_posts").fetchone()[0]
    mismatch = con.execute(f"""
        SELECT count(*) FROM current_posts c JOIN observations o ON c.row_id=o.row_id
        LEFT JOIN current_tag_state s ON s.post_id=c.post_id
        WHERE s.tag_fingerprint IS DISTINCT FROM sha256(to_json({tag_names("o.tag_string")}))
    """).fetchone()[0]
    if count != expected or mismatch:
        raise IntegrityError("当前标签投影的帖子覆盖或标签指纹不一致")
    # Materialize and order the independent reference once. EXCEPT ALL uses window
    # operators; bounding each comparison avoids sorting hundreds of millions of
    # rows in four global windows while retaining exact duplicate-sensitive checks.
    if emit:
        emit("tag_projection_reference_started")
    con.execute("""
        CREATE OR REPLACE TEMP TABLE tag_verification_expected AS
        SELECT c.post_id,t.tag_id FROM current_posts c JOIN tag_postings t ON c.row_id=t.row_id
        ORDER BY c.post_id,t.tag_id
    """)
    low, high, expected_tags = con.execute(
        "SELECT min(post_id),max(post_id),count(*) FROM tag_verification_expected"
    ).fetchone()
    if postings != expected_tags:
        raise IntegrityError("当前标签关系数量与历史观察不一致")
    if emit:
        emit("tag_projection_reference_finished", tag_postings=expected_tags)
    if low is not None:
        for start in range(low, high + 1, 250000):
            mismatch = con.execute(
                """
                WITH expected AS (
                    SELECT * FROM tag_verification_expected WHERE post_id>=? AND post_id<?
                ), actual AS (
                    SELECT * FROM current_tag_postings WHERE post_id>=? AND post_id<?
                ), differences AS (
                    (SELECT * FROM expected EXCEPT ALL SELECT * FROM actual)
                    UNION ALL
                    (SELECT * FROM actual EXCEPT ALL SELECT * FROM expected)
                ) SELECT count(*) FROM differences
            """,
                [start, start + 250000, start, start + 250000],
            ).fetchone()[0]
            if mismatch:
                raise IntegrityError(f"当前标签关系与历史观察不一致: post_id {start}–{start + 249999}")
            if emit:
                emit("tag_projection_verified_range", through_post_id=min(start + 249999, high))
    con.execute("DROP TABLE tag_verification_expected")
    return {"posts": count, "tag_postings": postings}
