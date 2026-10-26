//! The installed-font cache of the font pickers (ADR 0042, schema v22–v24): per font file its size, modification
//! time and the parse version that read it, per face its parsed names and metadata, and per listed family where its
//! name subset came from. Only names and numbers, never font bytes; the files are the truth and the cache is rebuilt
//! from them (`font_cache_apply`, `font_subsets_apply`).

use super::{Database, DatabaseError};
use rusqlite::params;

/// A font file as last parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFileStamp {
    pub path: String,
    pub size_bytes: u64,
    pub modified_at_utc_ms: i64,
    /// `player_fonts::PARSE_VERSION` of the parse that produced the file's faces (0: before v23). A file read by
    /// another version is reparsed.
    pub parse_version: u32,
}

/// One cached face of a font file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedFontFace {
    pub path: String,
    pub face_index: u32,
    /// The face's localized names as JSON (`player_fonts::FaceNames`).
    pub names_json: String,
    pub weight: u16,
    pub italic: bool,
    /// `player_fonts::Scripts` bits.
    pub scripts: u8,
    /// Not offered in the pickers (`player_fonts::FaceInfo::hidden`), kept so a saved setting still resolves.
    pub hidden: bool,
}

/// Where a listed family's name subset (ADR 0042) was made from. A record whose source stamp, display name or
/// subset version no longer matches is stale and the subset is made again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontSubsetRecord {
    /// The family's stored name, lower-case.
    pub family_key: String,
    pub source_path: String,
    pub face_index: u32,
    pub source_size_bytes: u64,
    pub source_modified_at_utc_ms: i64,
    /// `player_fonts::stable_hash` of the display name the subset holds.
    pub name_hash: String,
    /// `player_fonts::SUBSET_VERSION` of the subset.
    pub subset_version: u32,
    /// The subset's file name in the subset folder; `None`: the face cannot be subset (a legacy-encoding font), kept
    /// so it is not tried again until its source or name changes.
    pub subset_file: Option<String>,
}

impl Database {
    /// Every cached font file.
    pub fn font_cache_files(&self) -> Result<Vec<FontFileStamp>, DatabaseError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
            "SELECT path, size_bytes, modified_at_utc_ms, parse_version FROM font_files ORDER BY path",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(FontFileStamp {
                path: row.get(0)?,
                size_bytes: row.get::<_, i64>(1)?.max(0) as u64,
                modified_at_utc_ms: row.get(2)?,
                parse_version: row.get::<_, i64>(3)?.clamp(0, i64::from(u32::MAX)) as u32,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every cached face, in file and face order.
    pub fn font_cache_faces(&self) -> Result<Vec<CachedFontFace>, DatabaseError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT path, face_index, names_json, weight, italic, scripts, hidden FROM font_faces
             ORDER BY path, face_index",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(CachedFontFace {
                path: row.get(0)?,
                face_index: row.get::<_, i64>(1)?.clamp(0, i64::from(u32::MAX)) as u32,
                names_json: row.get(2)?,
                weight: row.get::<_, i64>(3)?.clamp(0, i64::from(u16::MAX)) as u16,
                italic: row.get::<_, i64>(4)? != 0,
                scripts: row.get::<_, i64>(5)?.clamp(0, 255) as u8,
                hidden: row.get::<_, i64>(6)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// In one transaction: forget the `removed` files and replace the cached faces of each `parsed` file (a file
    /// without usable faces is kept with no faces, so it is not reparsed until it changes).
    pub fn font_cache_apply(
        &self,
        removed: &[String],
        parsed: &[(FontFileStamp, Vec<CachedFontFace>)],
    ) -> Result<(), DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction()?;
        for path in removed
            .iter()
            .chain(parsed.iter().map(|(stamp, _)| &stamp.path))
        {
            tx.execute("DELETE FROM font_faces WHERE path=?1", [path])?;
            tx.execute("DELETE FROM font_files WHERE path=?1", [path])?;
        }
        for (stamp, faces) in parsed {
            tx.execute(
                "INSERT INTO font_files (path, size_bytes, modified_at_utc_ms, parse_version)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    stamp.path,
                    i64::try_from(stamp.size_bytes).unwrap_or(i64::MAX),
                    stamp.modified_at_utc_ms,
                    i64::from(stamp.parse_version)
                ],
            )?;
            for face in faces {
                tx.execute(
                    "INSERT OR REPLACE INTO font_faces
                         (path, face_index, names_json, weight, italic, scripts, hidden)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        stamp.path,
                        i64::from(face.face_index),
                        face.names_json,
                        i64::from(face.weight),
                        i64::from(face.italic),
                        i64::from(face.scripts),
                        i64::from(face.hidden)
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

impl Database {
    /// Every name subset record, by family.
    pub fn font_subsets(&self) -> Result<Vec<FontSubsetRecord>, DatabaseError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT family_key, source_path, face_index, source_size_bytes, source_modified_at_utc_ms, name_hash,
                    subset_version, subset_file
             FROM font_subsets ORDER BY family_key",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(FontSubsetRecord {
                family_key: row.get(0)?,
                source_path: row.get(1)?,
                face_index: row.get::<_, i64>(2)?.clamp(0, i64::from(u32::MAX)) as u32,
                source_size_bytes: row.get::<_, i64>(3)?.max(0) as u64,
                source_modified_at_utc_ms: row.get(4)?,
                name_hash: row.get(5)?,
                subset_version: row.get::<_, i64>(6)?.clamp(0, i64::from(u32::MAX)) as u32,
                subset_file: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// In one transaction: forget the subsets of the `removed` families and record (replace) `made`.
    pub fn font_subsets_apply(
        &self,
        removed: &[String],
        made: &[FontSubsetRecord],
    ) -> Result<(), DatabaseError> {
        let mut connection = self.lock()?;
        let tx = connection.transaction()?;
        for key in removed {
            tx.execute("DELETE FROM font_subsets WHERE family_key=?1", [key])?;
        }
        for record in made {
            tx.execute(
                "INSERT OR REPLACE INTO font_subsets
                     (family_key, source_path, face_index, source_size_bytes, source_modified_at_utc_ms, name_hash,
                      subset_version, subset_file)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    record.family_key,
                    record.source_path,
                    i64::from(record.face_index),
                    i64::try_from(record.source_size_bytes).unwrap_or(i64::MAX),
                    record.source_modified_at_utc_ms,
                    record.name_hash,
                    i64::from(record.subset_version),
                    record.subset_file
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(path: &str, size: u64) -> FontFileStamp {
        FontFileStamp {
            path: path.to_owned(),
            size_bytes: size,
            modified_at_utc_ms: 7,
            parse_version: 2,
        }
    }

    fn face(path: &str, index: u32, names: &str) -> CachedFontFace {
        CachedFontFace {
            path: path.to_owned(),
            face_index: index,
            names_json: names.to_owned(),
            weight: 700,
            italic: index == 1,
            scripts: 5,
            hidden: false,
        }
    }

    #[test]
    fn faces_are_replaced_per_file_and_removed_files_are_forgotten() {
        let db = Database::open_in_memory().expect("database");
        assert!(db.font_cache_files().unwrap().is_empty());
        db.font_cache_apply(
            &[],
            &[
                (
                    stamp("a.ttc", 10),
                    vec![face("a.ttc", 0, "A0"), face("a.ttc", 1, "A1")],
                ),
                (stamp("b.ttf", 20), vec![face("b.ttf", 0, "B")]),
                (stamp("broken.ttf", 1), vec![]),
            ],
        )
        .unwrap();
        assert_eq!(
            db.font_cache_files().unwrap(),
            vec![
                stamp("a.ttc", 10),
                stamp("b.ttf", 20),
                stamp("broken.ttf", 1)
            ]
        );
        assert_eq!(db.font_cache_faces().unwrap().len(), 3);
        assert_eq!(db.font_cache_faces().unwrap()[1], face("a.ttc", 1, "A1"));

        // a.ttc changed to one face; b.ttf vanished.
        db.font_cache_apply(
            &["b.ttf".to_owned()],
            &[(stamp("a.ttc", 11), vec![face("a.ttc", 0, "A0 new")])],
        )
        .unwrap();
        assert_eq!(
            db.font_cache_files().unwrap(),
            vec![stamp("a.ttc", 11), stamp("broken.ttf", 1)]
        );
        assert_eq!(
            db.font_cache_faces().unwrap(),
            vec![face("a.ttc", 0, "A0 new")]
        );
    }

    #[test]
    fn hidden_faces_and_parse_versions_round_trip() {
        let db = Database::open_in_memory().expect("database");
        let mut ext_b = face("mingliub.ttc", 0, "MingLiU-ExtB");
        ext_b.hidden = true;
        let shown = face("mingliub.ttc", 1, "Visible");
        db.font_cache_apply(
            &[],
            &[(stamp("mingliub.ttc", 5), vec![ext_b.clone(), shown.clone()])],
        )
        .unwrap();
        let faces = db.font_cache_faces().unwrap();
        assert_eq!(faces, vec![ext_b, shown.clone()]);
        assert_eq!(db.font_cache_files().unwrap()[0].parse_version, 2);
        // Reparsed by a later version: the flag follows the new parse.
        let mut newer = stamp("mingliub.ttc", 5);
        newer.parse_version = 3;
        db.font_cache_apply(&[], &[(newer.clone(), vec![shown.clone()])])
            .unwrap();
        assert_eq!(db.font_cache_files().unwrap(), vec![newer]);
        assert_eq!(db.font_cache_faces().unwrap(), vec![shown]);
    }

    #[test]
    fn migrating_to_v23_marks_the_cached_fonts_stale() {
        let db = Database::open_in_memory().expect("database");
        {
            let connection = db.lock().unwrap();
            connection
                .execute_batch("DROP TABLE font_faces; DROP TABLE font_files;")
                .unwrap();
            connection.execute_batch(super::super::SCHEMA_V22).unwrap();
            connection
                .execute_batch(
                    "INSERT INTO font_files VALUES ('old.ttf', 1, 2);
                     INSERT INTO font_faces VALUES ('old.ttf', 0, '{}', 400, 0, 0);",
                )
                .unwrap();
            connection.pragma_update(None, "user_version", 22).unwrap();
        }
        db.migrate().expect("migrate v22 to v23");
        db.migrate().expect("again: nothing to do");
        let files = db.font_cache_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].parse_version, 0, "parsed before v23: stale");
        assert!(!db.font_cache_faces().unwrap()[0].hidden);
    }

    fn subset_record(key: &str, file: Option<&str>) -> FontSubsetRecord {
        FontSubsetRecord {
            family_key: key.to_owned(),
            source_path: format!("{key}.ttf"),
            face_index: 1,
            source_size_bytes: 10,
            source_modified_at_utc_ms: 20,
            name_hash: "00ff".to_owned(),
            subset_version: 1,
            subset_file: file.map(str::to_owned),
        }
    }

    #[test]
    fn subset_records_are_replaced_and_forgotten() {
        let db = Database::open_in_memory().expect("database");
        assert!(db.font_subsets().unwrap().is_empty());
        let a = subset_record("a", Some("a1.ttf"));
        let legacy = subset_record("legacy", None);
        db.font_subsets_apply(&[], &[a.clone(), legacy.clone()])
            .unwrap();
        assert_eq!(db.font_subsets().unwrap(), vec![a, legacy.clone()]);
        let mut renamed = subset_record("a", Some("a2.ttf"));
        renamed.name_hash = "1234".to_owned();
        db.font_subsets_apply(&["legacy".to_owned()], std::slice::from_ref(&renamed))
            .unwrap();
        assert_eq!(db.font_subsets().unwrap(), vec![renamed]);
    }

    #[test]
    fn migrating_to_v24_adds_the_subset_table() {
        let db = Database::open_in_memory().expect("database");
        {
            let connection = db.lock().unwrap();
            connection
                .execute_batch("DROP TABLE font_subsets;")
                .unwrap();
            connection.pragma_update(None, "user_version", 23).unwrap();
        }
        db.migrate().expect("migrate v23 to v24");
        db.migrate().expect("again: nothing to do");
        db.lock()
            .unwrap()
            .execute_batch(super::super::SCHEMA_V24)
            .expect("re-entrant script");
        db.font_subsets_apply(&[], &[subset_record("b", Some("b.otf"))])
            .unwrap();
        assert_eq!(db.font_subsets().unwrap().len(), 1);
    }

    #[test]
    fn migrating_to_v22_adds_the_font_tables_and_is_reentrant() {
        let db = Database::open_in_memory().expect("database");
        {
            let connection = db.lock().unwrap();
            connection
                .execute_batch("DROP TABLE font_faces; DROP TABLE font_files;")
                .unwrap();
            connection.pragma_update(None, "user_version", 21).unwrap();
        }
        db.migrate().expect("migrate v21 to v22");
        db.migrate().expect("again: nothing to do");
        // Re-running the v22 script on a database that already has the tables is harmless.
        db.lock()
            .unwrap()
            .execute_batch(super::super::SCHEMA_V22)
            .expect("re-entrant script");
        let version: i64 = db
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, super::super::SCHEMA_VERSION);
        db.font_cache_apply(&[], &[(stamp("c.otf", 3), vec![face("c.otf", 0, "C")])])
            .unwrap();
        assert_eq!(db.font_cache_faces().unwrap().len(), 1);
    }
}
