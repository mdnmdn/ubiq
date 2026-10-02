//! Pending row edits: what a table tab's grid holds between "the user changed a cell" and "Apply".
//!
//! No widget and no message here. The grid keeps the rows exactly as they were loaded; this module
//! keeps what differs — replaced cells (`overlay`), the state of each touched row (`marks`) — and
//! turns it into [`RowEdit`]s for `ubiq_db::edit` to render. An update or delete is keyed on the
//! row **as loaded**, never on the pending values.

use std::collections::HashMap;

use ubiq_db::edit::key_for_row;
use ubiq_proto::db::{ColumnMeta, RowEdit, Value};

/// What a row is doing, as the grid draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowMark {
    #[default]
    Clean,
    /// Has pending cell changes in the overlay.
    Edited,
    /// A new row not yet in the database.
    Inserted,
    /// Marked for deletion.
    Deleted,
}

#[derive(Default)]
pub struct Pending {
    /// Rows `0..loaded` came from the database; anything after was appended with [`Self::add_row`].
    loaded: usize,
    marks: HashMap<usize, RowMark>,
    overlay: HashMap<(usize, usize), Value>,
}

impl Pending {
    /// Forget every pending change; `loaded` rows are now the database's.
    pub fn reset(&mut self, loaded: usize) {
        self.loaded = loaded;
        self.marks.clear();
        self.overlay.clear();
    }

    pub fn loaded(&self) -> usize {
        self.loaded
    }

    pub fn mark(&self, row: usize) -> RowMark {
        self.marks.get(&row).copied().unwrap_or_default()
    }

    pub fn overlay(&self, row: usize, col: usize) -> Option<&Value> {
        self.overlay.get(&(row, col))
    }

    /// Rows with anything pending.
    pub fn count(&self) -> usize {
        self.marks.len()
    }

    /// Set a cell to `value`; a value equal to the `original` drops the overlay again.
    pub fn set(&mut self, row: usize, col: usize, value: Value, original: &Value) {
        if &value == original {
            self.overlay.remove(&(row, col));
        } else {
            self.overlay.insert((row, col), value);
        }
        if !matches!(self.mark(row), RowMark::Inserted | RowMark::Deleted) {
            self.settle(row);
        }
    }

    /// Row `row` (the grid's new last row) is a new row.
    pub fn add_row(&mut self, row: usize) {
        self.marks.insert(row, RowMark::Inserted);
    }

    /// Mark a row deleted, or take the mark back.
    pub fn toggle_delete(&mut self, row: usize) {
        if self.mark(row) == RowMark::Deleted {
            if row >= self.loaded {
                self.marks.insert(row, RowMark::Inserted);
            } else {
                self.settle(row);
            }
        } else {
            self.marks.insert(row, RowMark::Deleted);
        }
    }

    /// An existing row is `Edited` while any cell of it is overlaid, else clean.
    fn settle(&mut self, row: usize) {
        if self.overlay.keys().any(|(r, _)| *r == row) {
            self.marks.insert(row, RowMark::Edited);
        } else {
            self.marks.remove(&row);
        }
    }

    /// The edits, in row order. A deleted new row is nothing; an inserted row carries only the
    /// cells the user set (the rest take the column's default).
    pub fn edits(&self, columns: &[ColumnMeta], rows: &[Vec<Value>]) -> Vec<RowEdit> {
        self.edits_by_row(columns, rows)
            .into_iter()
            .map(|(_, edit)| edit)
            .collect()
    }

    /// [`Self::edits`], each with the row it came from — how a failing statement's index in the
    /// batch finds the row to mark.
    pub fn edits_by_row(
        &self,
        columns: &[ColumnMeta],
        rows: &[Vec<Value>],
    ) -> Vec<(usize, RowEdit)> {
        let mut out = Vec::new();
        for (ix, row) in rows.iter().enumerate() {
            let cells = |keep: &dyn Fn(usize) -> bool| -> Vec<(ColumnMeta, Value)> {
                columns
                    .iter()
                    .enumerate()
                    .filter(|(col, _)| keep(*col))
                    .filter_map(|(col, meta)| {
                        self.overlay
                            .get(&(ix, col))
                            .map(|v| (meta.clone(), v.clone()))
                    })
                    .collect()
            };
            match self.mark(ix) {
                RowMark::Clean => {}
                RowMark::Deleted if ix >= self.loaded => {}
                RowMark::Deleted => out.push((
                    ix,
                    RowEdit::Delete {
                        key: key_for_row(columns, row),
                    },
                )),
                RowMark::Inserted => out.push((
                    ix,
                    RowEdit::Insert {
                        values: cells(&|_| true),
                    },
                )),
                RowMark::Edited => {
                    let changes = cells(&|col| row.get(col) != self.overlay.get(&(ix, col)));
                    if !changes.is_empty() {
                        out.push((
                            ix,
                            RowEdit::Update {
                                key: key_for_row(columns, row),
                                changes,
                            },
                        ));
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::db::DbKind;

    fn cols() -> Vec<ColumnMeta> {
        let mut id = ColumnMeta::new(DbKind::Sqlite, "id", "INTEGER");
        id.is_pk = true;
        vec![id, ColumnMeta::new(DbKind::Sqlite, "name", "TEXT")]
    }

    fn rows() -> Vec<Vec<Value>> {
        vec![
            vec![Value::Int(1), Value::Text("a".into())],
            vec![Value::Int(2), Value::Text("b".into())],
        ]
    }

    fn pending() -> Pending {
        let mut p = Pending::default();
        p.reset(2);
        p
    }

    #[test]
    fn update_is_keyed_on_the_loaded_row() {
        let (cols, rows) = (cols(), rows());
        let mut p = pending();
        // Change the key and another cell: the key stays the loaded one.
        p.set(0, 0, Value::Int(9), &rows[0][0]);
        p.set(0, 1, Value::Text("z".into()), &rows[0][1]);
        assert_eq!(p.mark(0), RowMark::Edited);
        let edits = p.edits(&cols, &rows);
        let [RowEdit::Update { key, changes }] = edits.as_slice() else {
            panic!("{edits:?}");
        };
        assert_eq!(key.len(), 1);
        assert_eq!(key[0].1, Value::Int(1));
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn setting_the_original_back_settles_the_row() {
        let (cols, rows) = (cols(), rows());
        let mut p = pending();
        p.set(1, 1, Value::Text("x".into()), &rows[1][1]);
        p.set(1, 1, rows[1][1].clone(), &rows[1][1]);
        assert_eq!(p.mark(1), RowMark::Clean);
        assert_eq!(p.count(), 0);
        assert!(p.edits(&cols, &rows).is_empty());
    }

    #[test]
    fn delete_toggles_and_keys_on_the_loaded_row() {
        let (cols, rows) = (cols(), rows());
        let mut p = pending();
        p.set(1, 1, Value::Text("x".into()), &rows[1][1]);
        p.toggle_delete(1);
        assert_eq!(p.mark(1), RowMark::Deleted);
        let edits = p.edits(&cols, &rows);
        assert!(matches!(edits.as_slice(), [RowEdit::Delete { key }] if key[0].1 == Value::Int(2)));
        p.toggle_delete(1);
        assert_eq!(p.mark(1), RowMark::Edited);
        p.set(1, 1, rows[1][1].clone(), &rows[1][1]);
        p.toggle_delete(1);
        p.toggle_delete(1);
        assert_eq!(p.mark(1), RowMark::Clean);
    }

    #[test]
    fn insert_carries_only_set_cells_and_a_deleted_insert_is_nothing() {
        let cols = cols();
        let mut rows = rows();
        let mut p = pending();
        rows.push(vec![Value::Null, Value::Null]);
        p.add_row(2);
        p.set(2, 1, Value::Text("n".into()), &Value::Null);
        let edits = p.edits(&cols, &rows);
        let [RowEdit::Insert { values }] = edits.as_slice() else {
            panic!("{edits:?}");
        };
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].0.name, "name");
        p.toggle_delete(2);
        assert!(p.edits(&cols, &rows).is_empty());
        p.toggle_delete(2);
        assert_eq!(p.mark(2), RowMark::Inserted);
    }

    #[test]
    fn each_edit_names_the_row_it_came_from() {
        let (cols, mut rows) = (cols(), rows());
        let mut p = pending();
        p.set(1, 1, Value::Text("x".into()), &rows[1][1]);
        rows.push(vec![Value::Null, Value::Null]);
        p.add_row(2);
        p.set(2, 1, Value::Text("n".into()), &Value::Null);
        let by_row = p.edits_by_row(&cols, &rows);
        assert_eq!(
            by_row.iter().map(|(row, _)| *row).collect::<Vec<_>>(),
            [1, 2]
        );
    }
}
