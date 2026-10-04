//! SQLite adapter for fixed ranking recipes. Existing selection, workset and
//! task SQL can consume members without owning another copy of their keys.
use crate::{
    db_error,
    ranking_projection::{RankingProjection, RankingProjectionReader},
};
use rusqlite::{
    Connection, Result as SqlResult, ffi,
    types::Value,
    vtab::{
        Context, Filters, IndexConstraintOp as Op, IndexInfo, Module, VTab, VTabConnection,
        VTabCursor,
    },
};
use std::{
    borrow::Cow,
    ffi::{CStr, c_int},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

pub(super) fn load(db: &Connection, directory: &std::path::Path) -> studio_domain::Result<()> {
    const MODULE: Module<Members> = Module::eponymous_only_module();
    db.create_module(c"ranking_members", &MODULE, Some(directory.to_path_buf()))
        .map_err(db_error)
}
fn sql_error(error: impl std::fmt::Display) -> rusqlite::Error {
    rusqlite::Error::ModuleError(error.to_string())
}

#[repr(C)]
struct Members {
    // SQLite requires the base header at offset zero.
    base: ffi::sqlite3_vtab,
    directory: PathBuf,
}

// SAFETY: repr(C) and the leading sqlite3_vtab header satisfy rusqlite's ABI.
// Every cursor owns its connections and buffers; it never borrows SQLite rows.
unsafe impl<'vtab> VTab<'vtab> for Members {
    type Aux = PathBuf;
    type Cursor = MemberCursor;
    fn connect(
        _: &mut VTabConnection,
        directory: Option<&PathBuf>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> SqlResult<(Cow<'static, CStr>, Self)> {
        Ok((Cow::Borrowed(c"CREATE TABLE x(source_id TEXT,asset_id TEXT,ordinal INTEGER,post_id INTEGER,matched INTEGER,recipe HIDDEN)"), Self {base:ffi::sqlite3_vtab::default(), directory:directory.cloned().ok_or_else(||sql_error("missing ranking member root"))?}))
    }
    fn best_index(&self, info: &mut IndexInfo) -> SqlResult<bool> {
        let mut constraints = Vec::new();
        let mut recipe = false;
        let mut point = false;
        for (constraint, mut usage) in info.constraints_and_usages() {
            if !constraint.is_usable() {
                continue;
            }
            let column = constraint.column();
            let op = match constraint.operator() {
                Op::SQLITE_INDEX_CONSTRAINT_EQ => "=",
                Op::SQLITE_INDEX_CONSTRAINT_GT => ">",
                Op::SQLITE_INDEX_CONSTRAINT_GE => ">=",
                Op::SQLITE_INDEX_CONSTRAINT_LT => "<",
                Op::SQLITE_INDEX_CONSTRAINT_LE => "<=",
                _ => continue,
            };
            if (column == 5 && op == "=") || matches!(column, 0 | 1) {
                recipe |= column == 5;
                point |= column == 1 && op == "=";
                constraints.push(format!("{column}{op}"));
                usage.set_argv_index(constraints.len() as c_int);
                usage.set_omit(true);
            }
        }
        if !recipe {
            return Ok(false);
        }
        let orders = info
            .order_bys()
            .map(|o| (o.column(), o.is_order_by_desc()))
            .collect::<Vec<_>>();
        let source_equal = constraints.iter().any(|v| v == "0=");
        let ordered = matches!(orders.as_slice(), [(0, _)])
            || matches!(orders.as_slice(),[(0,a),(1,b)] if a==b)
            || (source_equal && matches!(orders.as_slice(), [(1, _)]));
        let descending = ordered && orders[0].1;
        info.set_idx_num(i32::from(descending));
        info.set_idx_str(&constraints.join(","));
        info.set_order_by_consumed(ordered);
        info.set_estimated_cost(if point { 1.0 } else { 10_000.0 });
        info.set_estimated_rows(if point { 1 } else { 1_000_000 });
        Ok(true)
    }
    fn open(&mut self) -> SqlResult<MemberCursor> {
        Ok(MemberCursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            directory: self.directory.clone(),
            reader: None,
            sources: Vec::new(),
            source: 0,
            constraints: Vec::new(),
            after: None,
            descending: false,
            rows: Vec::new(),
            row: 0,
            sequence: 0,
            recipe: String::new(),
        })
    }
}

struct Member {
    source: String,
    asset: String,
    ordinal: i64,
    post: Option<i64>,
    matched: bool,
}

#[repr(C)]
struct MemberCursor {
    base: ffi::sqlite3_vtab_cursor,
    directory: PathBuf,
    reader: Option<RankingProjectionReader>,
    sources: Vec<String>,
    source: usize,
    constraints: Vec<(String, String)>,
    after: Option<String>,
    descending: bool,
    rows: Vec<Member>,
    row: usize,
    sequence: i64,
    recipe: String,
}
impl MemberCursor {
    fn small(&mut self) -> SqlResult<bool> {
        let Some(reader) = &self.reader else {
            return Ok(false);
        };
        if reader.projection.count > 4096 {
            return Ok(false);
        }
        let (predicate, values) = reader.predicate().map_err(sql_error)?;
        let sql = format!(
            "SELECT i.source_id,i.asset_id,i.ordinal,i.post_id FROM (SELECT ordinal FROM scores WHERE {predicate} LIMIT 4097) m JOIN fixed_input.input_rows i ON i.ordinal=m.ordinal"
        );
        self.rows = reader
            .db
            .prepare(&sql)?
            .query_map(rusqlite::params_from_iter(values), |r| {
                Ok(Member {
                    source: r.get(0)?,
                    asset: hex::encode(r.get::<_, Vec<u8>>(1)?),
                    ordinal: r.get(2)?,
                    post: r.get(3)?,
                    matched: true,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        if self.rows.len() > 4096 {
            return Err(sql_error(
                "small ranking member count does not match its recipe",
            ));
        }
        self.rows.retain(|row| {
            self.sources.contains(&row.source)
                && self
                    .constraints
                    .iter()
                    .all(|(op, value)| match op.as_str() {
                        "=" => &row.asset == value,
                        ">" => &row.asset > value,
                        ">=" => &row.asset >= value,
                        "<" => &row.asset < value,
                        "<=" => &row.asset <= value,
                        _ => false,
                    })
        });
        self.rows
            .sort_by(|a, b| (&a.source, &a.asset).cmp(&(&b.source, &b.asset)));
        if self.descending {
            self.rows.reverse();
        }
        self.reader = None;
        Ok(true)
    }
    fn fill(&mut self) -> SqlResult<()> {
        self.rows.clear();
        self.row = 0;
        let Some(reader) = &self.reader else {
            return Ok(());
        };
        while let Some(source) = self.sources.get(self.source) {
            let (predicate, mut values) = reader.predicate().map_err(sql_error)?;
            // Return the match flag as a column instead of hiding rejected
            // candidates inside xNext. SQLite's outer VM can then enforce its
            // normal cancellation/progress callback even for a very sparse set.
            let matched = if predicate == "1=1" {
                "1".into()
            } else {
                format!(
                    "EXISTS(SELECT 1 FROM main.scores WHERE ordinal=i.ordinal AND ({predicate}))"
                )
            };
            values.push(Value::Text(source.clone()));
            let mut clauses = vec![format!("i.source_id=?{}", values.len())];
            for (operator, value) in &self.constraints {
                // Asset keys are canonical lowercase SHA-256 strings. Keeping
                // comparisons as bytes preserves their BINARY text order.
                let bytes = hex::decode(value).map_err(sql_error)?;
                values.push(Value::Blob(bytes));
                clauses.push(format!("i.asset_id{operator}?{}", values.len()));
            }
            let direction = if self.descending { "DESC" } else { "ASC" };
            if let Some(after) = &self.after {
                values.push(Value::Blob(hex::decode(after).map_err(sql_error)?));
                clauses.push(format!(
                    "i.asset_id{}?{}",
                    if self.descending { "<" } else { ">" },
                    values.len()
                ));
            }
            let sql = format!(
                "SELECT i.source_id,i.asset_id,i.ordinal,i.post_id,{matched} FROM fixed_input.input_rows i INDEXED BY input_identity WHERE {} ORDER BY i.asset_id {direction} LIMIT 512",
                clauses.join(" AND ")
            );
            self.rows = reader
                .db
                .prepare(&sql)?
                .query_map(rusqlite::params_from_iter(values), |r| {
                    Ok(Member {
                        source: r.get(0)?,
                        asset: hex::encode(r.get::<_, Vec<u8>>(1)?),
                        ordinal: r.get(2)?,
                        post: r.get(3)?,
                        matched: r.get(4)?,
                    })
                })?
                .collect::<SqlResult<Vec<_>>>()?;
            if let Some(last) = self.rows.last() {
                self.after = Some(last.asset.clone());
                return Ok(());
            }
            self.source += 1;
            self.after = None;
        }
        Ok(())
    }
}
// SAFETY: repr(C), a leading sqlite3_vtab_cursor header, and fully owned data
// meet the VTabCursor ABI. No raw pointers or borrowed statement rows escape.
unsafe impl VTabCursor for MemberCursor {
    fn filter(
        &mut self,
        idx_num: c_int,
        idx_str: Option<&str>,
        args: &Filters<'_>,
    ) -> SqlResult<()> {
        self.reader = None;
        self.sources.clear();
        self.source = 0;
        self.constraints.clear();
        self.after = None;
        self.rows.clear();
        self.row = 0;
        self.sequence = 0;
        self.recipe.clear();
        self.descending = idx_num != 0;
        let mut source_constraints = Vec::new();
        for (index, part) in idx_str.unwrap_or_default().split(',').enumerate() {
            let Some(value) = args.get::<Option<String>>(index)? else {
                return Ok(());
            };
            if part == "5=" {
                self.recipe = value;
            } else if let Some(operator) = part.strip_prefix('0') {
                source_constraints.push((operator.to_string(), value));
            } else if let Some(operator) = part.strip_prefix('1') {
                // Non-key bounds such as the empty pagination sentinel must be
                // interpreted as text, not passed to hex decoding.
                if value.is_empty() && matches!(operator, ">" | ">=") {
                    continue;
                }
                if value == "\u{10ffff}" && matches!(operator, "<" | "<=") {
                    continue;
                }
                if value.len() != 64
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    if operator == "=" || (value.is_empty() && matches!(operator, "<" | "<=")) {
                        return Ok(());
                    }
                    return Err(sql_error("noncanonical ranking member bound"));
                }
                self.constraints.push((operator.to_string(), value));
            }
        }
        if self.recipe.len() > 65_536 {
            return Err(sql_error("ranking recipe exceeds its bound"));
        }
        let projection: RankingProjection =
            serde_json::from_str(&self.recipe).map_err(sql_error)?;
        self.sources = projection.source_ids.clone();
        self.sources.sort();
        self.sources.dedup();
        self.sources.retain(|source| {
            source_constraints
                .iter()
                .all(|(op, value)| match op.as_str() {
                    "=" => source == value,
                    ">" => source > value,
                    ">=" => source >= value,
                    "<" => source < value,
                    "<=" => source <= value,
                    _ => false,
                })
        });
        if self.descending {
            self.sources.reverse();
        }
        if self.sources.is_empty() || projection.count == 0 {
            return Ok(());
        }
        self.reader = Some(
            RankingProjectionReader::open(
                &self.directory,
                &projection,
                Arc::new(AtomicBool::new(false)),
            )
            .map_err(sql_error)?,
        );
        if self.small()? {
            return Ok(());
        }
        self.fill()
    }
    fn next(&mut self) -> SqlResult<()> {
        self.row += 1;
        self.sequence += 1;
        if self.row >= self.rows.len() {
            self.fill()?;
        }
        Ok(())
    }
    fn eof(&self) -> bool {
        self.row >= self.rows.len()
    }
    fn column(&self, context: &mut Context, column: c_int) -> SqlResult<()> {
        let row = &self.rows[self.row];
        match column {
            0 => context.set_result(&row.source),
            1 => context.set_result(&row.asset),
            2 => context.set_result(&row.ordinal),
            3 => context.set_result(&row.post),
            4 => context.set_result(&row.matched),
            5 => context.set_result(&self.recipe),
            _ => Err(sql_error("unknown ranking member column")),
        }
    }
    fn rowid(&self) -> SqlResult<i64> {
        Ok(self.sequence)
    }
}
