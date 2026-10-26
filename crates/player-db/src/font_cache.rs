//! The installed-font cache of the font pickers (ADR 0042, schema v22): per font file its size and modification time,
//! per face its parsed names and metadata. Only names and numbers, never font bytes; the files are the truth and the
//! cache is rebuilt from them (`font_cache_apply`).

use super::{Database, DatabaseError};
use rusqlite::params;

/// A font file as last parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFileStamp {
    pub path: String,
    pub size_bytes: u64,
    pub modified_at_utc_ms: i64,
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
}

impl Database {
    /// Every cached font file.
    pub fn font_cache_files(&self) -> Result<Vec<FontFileStamp>, DatabaseError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT path, size_bytes, modified_at_utc_ms FROM font_files ORDER BY path")?;
        let rows = statement.query_map([], |row| {
            Ok(FontFileStamp {
                path: row.get(0)?,
                size_bytes: row.get::<_, i64>(1)?.max(0) as u64,
                modified_at_utc_ms: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every cached face, in file and face order.
    pub fn font_cache_faces(&self) -> Result<Vec<CachedFontFace>, DatabaseError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT path, face_index, names_json, weight, italic, scripts FROM font_faces
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
                "INSERT INTO font_files (path, size_bytes, modified_at_utc_ms) VALUES (?1, ?2, ?3)",
                params![
                    stamp.path,
                    i64::try_from(stamp.size_bytes).unwrap_or(i64::MAX),
                    stamp.modified_at_utc_ms
                ],
            )?;
            for face in faces {
                tx.execute(
                    "INSERT OR REPLACE INTO font_faces (path, face_index, names_json, weight, italic, scripts)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        stamp.path,
                        i64::from(face.face_index),
                        face.names_json,
                        i64::from(face.weight),
                        i64::from(face.italic),
                        i64::from(face.scripts)
                    ],
                )?;
            }
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
