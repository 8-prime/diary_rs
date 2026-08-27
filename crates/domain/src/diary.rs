use std::collections::{HashMap, HashSet};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use jiff::{Timestamp, tz::TimeZone};
use rand::Rng;
use sqlx::SqlitePool;

use crate::images::ImageService;
use crate::{Error, Result};

pub struct Diary {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub share_token: String,
    pub timezone: String,
    pub created_at: i64,
}

pub struct DiaryView {
    pub diary: Diary,
    pub days: Vec<Day>,
}

pub struct Day {
    pub local_date: String,
    pub entries: Vec<EntryView>,
}

pub struct Entry {
    pub id: i64,
    pub diary_id: i64,
    pub local_date: String,
    pub occurred_at: i64,
    pub created_at: i64,
    pub text: Option<String>,
}
pub struct EntryView {
    pub entry: Entry,
    pub images: Vec<Image>,
}

pub struct Image {
    pub id: i64,
    pub entry_id: i64,
    pub hash: String,
    pub width: i64,
    pub height: i64,
    pub position: i64,
    pub alt: Option<String>,
}

pub struct NewDiary {
    pub title: String,
    pub description: Option<String>,
    pub timezone: String,
}

pub struct NewEntry {
    pub occurred_at: i64,
    pub text: Option<String>,
    pub images: Vec<NewImage>,
}

pub struct NewImage {
    pub hash: String,
    pub width: u32,
    pub height: u32,
    pub alt: Option<String>,
}

pub struct DiaryService {
    write: SqlitePool,
    read: SqlitePool,
    images: ImageService,
}

impl DiaryService {
    pub fn new(write: SqlitePool, read: SqlitePool, images: ImageService) -> DiaryService {
        return DiaryService {
            write,
            read,
            images,
        };
    }

    // Gets all diary details (not days though)
    pub async fn get_diaries(&self) -> Result<Vec<Diary>> {
        let diaries = sqlx::query_as!(
            Diary,
            "SELECT id, title, description, share_token, timezone, created_at FROM diary"
        )
        .fetch_all(&self.read)
        .await?;

        return Ok(diaries);
    }

    // Gets one full diary
    pub async fn get_diary(&self, id: i64) -> Result<DiaryView> {
        let diary = sqlx::query_as!(
            Diary,
            "SELECT id, title, description, share_token, timezone, created_at
             FROM diary
             WHERE id = ?",
            id
        )
        .fetch_optional(&self.read)
        .await?
        .ok_or(Error::NotFound)?;

        return self.load_view(diary).await;
    }

    pub async fn get_shared_diary(&self, share_token: &str) -> Result<DiaryView> {
        let diary = sqlx::query_as!(
            Diary,
            "SELECT id, title, description, share_token, timezone, created_at
             FROM diary
             WHERE share_token = ?",
            share_token
        )
        .fetch_optional(&self.read)
        .await?
        .ok_or(Error::NotFound)?;

        return self.load_view(diary).await;
    }

    async fn load_view(&self, diary: Diary) -> Result<DiaryView> {
        let id = diary.id;

        let entries = sqlx::query_as!(
            Entry,
            "SELECT id, diary_id, local_date, occurred_at, created_at, text
             FROM entry
             WHERE diary_id = ?
             ORDER BY local_date DESC, occurred_at ASC",
            id
        )
        .fetch_all(&self.read)
        .await?;

        // Joining back to entry avoids binding a list of ids, which SQLite
        // can't do and `query_as!` couldn't check anyway.
        let images = sqlx::query_as!(
            Image,
            "SELECT image.id, image.entry_id, image.hash, image.width,
                    image.height, image.position, image.alt
             FROM image
             JOIN entry ON entry.id = image.entry_id
             WHERE entry.diary_id = ?
             ORDER BY image.position ASC",
            id
        )
        .fetch_all(&self.read)
        .await?;

        let mut images_by_entry: HashMap<i64, Vec<Image>> = HashMap::new();
        for image in images {
            images_by_entry
                .entry(image.entry_id)
                .or_default()
                .push(image);
        }

        // `entries` is already ordered by day, so a new day starts whenever
        // local_date changes -- no grouping map needed, and order is kept.
        let mut days: Vec<Day> = Vec::new();
        for entry in entries {
            let images = images_by_entry.remove(&entry.id).unwrap_or_default();
            let local_date = entry.local_date.clone();
            let view = EntryView { entry, images };

            match days.last_mut() {
                Some(day) if day.local_date == local_date => day.entries.push(view),
                _ => days.push(Day {
                    local_date,
                    entries: vec![view],
                }),
            }
        }

        return Ok(DiaryView { diary, days });
    }

    // Creates a new diary entry and generates required data
    // (keys and id)
    pub async fn create_diary(&self, new: NewDiary) -> Result<Diary> {
        // Rejected up front so a typo cannot produce a diary whose entries can
        // never be dated.
        let _ = TimeZone::get(&new.timezone)?;

        let share_token = generate_share_token();
        let created_at = Timestamp::now().as_second();

        let diary = sqlx::query_as!(
            Diary,
            "INSERT INTO diary (title, description, share_token, timezone, created_at)
             VALUES (?, ?, ?, ?, ?)
             RETURNING id, title, description, share_token, timezone, created_at",
            new.title,
            new.description,
            share_token,
            new.timezone,
            created_at
        )
        .fetch_one(&self.write)
        .await?;

        return Ok(diary);
    }

    // Updates title and description of diary
    pub async fn update_diary(
        &self,
        id: i64,
        title: &str,
        description: Option<&str>,
    ) -> Result<Diary> {
        let diary = sqlx::query_as!(
            Diary,
            "UPDATE diary
             SET title = ?, description = ?
             WHERE id = ?
             RETURNING id, title, description, share_token, timezone, created_at",
            title,
            description,
            id
        )
        .fetch_optional(&self.write)
        .await?
        .ok_or(Error::NotFound)?;

        return Ok(diary);
    }

    // delete diary and entries
    pub async fn delete_diary(&self, id: i64) -> Result<()> {
        // entry and image rows go with it via ON DELETE CASCADE, which only
        // fires if the connection has PRAGMA foreign_keys on.
        let deleted = sqlx::query!("DELETE FROM diary WHERE id = ?", id)
            .execute(&self.write)
            .await?;

        if deleted.rows_affected() == 0 {
            return Err(Error::NotFound);
        }

        return Ok(());
    }

    // rerolls the public access link id
    pub async fn reroll_diary_keys(&self, id: i64) -> Result<String> {
        let share_token = generate_share_token();

        let updated = sqlx::query!(
            "UPDATE diary SET share_token = ? WHERE id = ?",
            share_token,
            id
        )
        .execute(&self.write)
        .await?;

        if updated.rows_affected() == 0 {
            return Err(Error::NotFound);
        }

        return Ok(share_token);
    }

    /// The timezone entries in this diary are dated in. Handlers need it to
    /// read a wall-clock time off a form.
    pub async fn diary_timezone(&self, diary_id: i64) -> Result<String> {
        let timezone = sqlx::query_scalar!("SELECT timezone FROM diary WHERE id = ?", diary_id)
            .fetch_optional(&self.read)
            .await?
            .ok_or(Error::NotFound)?;

        return Ok(timezone);
    }

    pub async fn create_entry(&self, diary_id: i64, new: NewEntry) -> Result<EntryView> {
        let timezone = self.diary_timezone(diary_id).await?;
        let local_date = local_date_for(&timezone, new.occurred_at)?;
        let created_at = Timestamp::now().as_second();

        // Checked before anything is written, so a hash with no blob behind it
        // fails the request instead of becoming a permanently broken image.
        for image in &new.images {
            if !self.images.exists(&image.hash) {
                return Err(Error::NotFound);
            }
        }

        let mut tx = self.write.begin().await?;

        let entry = sqlx::query_as!(
            Entry,
            // SQLite reports every RETURNING column as nullable, so the NOT
            // NULL ones are asserted back with `!`.
            r#"INSERT INTO entry (diary_id, local_date, occurred_at, created_at, text)
               VALUES (?, ?, ?, ?, ?)
               RETURNING id AS "id!", diary_id AS "diary_id!",
                         local_date AS "local_date!", occurred_at AS "occurred_at!",
                         created_at AS "created_at!", text"#,
            diary_id,
            local_date,
            new.occurred_at,
            created_at,
            new.text
        )
        .fetch_one(&mut *tx)
        .await?;

        let mut images = Vec::with_capacity(new.images.len());
        for (position, image) in new.images.into_iter().enumerate() {
            let position = position as i64;
            let width = image.width as i64;
            let height = image.height as i64;

            let row = sqlx::query_as!(
                Image,
                r#"INSERT INTO image (entry_id, hash, width, height, position, alt)
                   VALUES (?, ?, ?, ?, ?, ?)
                   RETURNING id AS "id!", entry_id AS "entry_id!", hash AS "hash!",
                             width AS "width!", height AS "height!",
                             position AS "position!", alt"#,
                entry.id,
                image.hash,
                width,
                height,
                position,
                image.alt
            )
            .fetch_one(&mut *tx)
            .await?;

            images.push(row);
        }

        tx.commit().await?;

        return Ok(EntryView { entry, images });
    }

    /// One entry on its own. Handlers use it to find the diary an entry
    /// belongs to, which is where they have to redirect back to.
    pub async fn get_entry(&self, entry_id: i64) -> Result<Entry> {
        let entry = sqlx::query_as!(
            Entry,
            "SELECT id, diary_id, local_date, occurred_at, created_at, text
             FROM entry
             WHERE id = ?",
            entry_id
        )
        .fetch_optional(&self.read)
        .await?
        .ok_or(Error::NotFound)?;

        return Ok(entry);
    }

    // update diary text
    pub async fn update_entry(&self, entry_id: i64, text: Option<&str>) -> Result<Entry> {
        let entry = sqlx::query_as!(
            Entry,
            "UPDATE entry
             SET text = ?
             WHERE id = ?
             RETURNING id, diary_id, local_date, occurred_at, created_at, text",
            text,
            entry_id
        )
        .fetch_optional(&self.write)
        .await?
        .ok_or(Error::NotFound)?;

        return Ok(entry);
    }

    pub async fn delete_entry(&self, entry_id: i64) -> Result<()> {
        let deleted = sqlx::query!("DELETE FROM entry WHERE id = ?", entry_id)
            .execute(&self.write)
            .await?;

        if deleted.rows_affected() == 0 {
            return Err(Error::NotFound);
        }

        return Ok(());
    }

    /// Removes the row and returns the entry it belonged to. The blob stays:
    /// another entry may share the hash, and the sweep is what reclaims it.
    pub async fn delete_photo(&self, image_id: i64) -> Result<i64> {
        let deleted = sqlx::query_scalar!(
            r#"DELETE FROM image WHERE id = ? RETURNING entry_id AS "entry_id!""#,
            image_id
        )
        .fetch_optional(&self.write)
        .await?
        .ok_or(Error::NotFound)?;

        return Ok(deleted);
    }

    /// Every blob hash still pointed at by a row. The blob sweep needs this to
    /// know what it is allowed to delete.
    pub async fn referenced_hashes(&self) -> Result<HashSet<String>> {
        let hashes = sqlx::query_scalar!("SELECT DISTINCT hash FROM image")
            .fetch_all(&self.read)
            .await?
            .into_iter()
            .collect();

        return Ok(hashes);
    }
}

/// 256 bits of URL-safe randomness. This is the only thing guarding the public
/// page of a diary, so it has to be unguessable rather than merely unique.
fn generate_share_token() -> String {
    let mut raw = [0u8; 32];
    rand::rng().fill_bytes(&mut raw);
    return URL_SAFE_NO_PAD.encode(raw);
}

/// Reads the value of an `<input type="datetime-local">` as a wall-clock time
/// in the diary's own timezone. The browser sends no offset, so without the
/// timezone the same string would mean different instants to different people.
pub fn timestamp_from_local(timezone: &str, local: &str) -> Result<i64> {
    let tz = TimeZone::get(timezone)?;

    // The seconds are optional in what the browser sends, and jiff wants a
    // complete civil time.
    let padded = if local.len() == 16 {
        format!("{local}:00")
    } else {
        local.to_string()
    };

    let civil: jiff::civil::DateTime = padded.parse()?;

    // A wall-clock time can be ambiguous across a DST change; taking the
    // earlier of the two is a better answer than refusing the entry.
    return Ok(civil
        .to_zoned(tz)
        .map_err(crate::Error::Time)?
        .timestamp()
        .as_second());
}

/// The calendar day an instant falls on for the people reading the diary,
/// which is why it is stored rather than computed at render time.
fn local_date_for(timezone: &str, occurred_at: i64) -> Result<String> {
    let tz = TimeZone::get(timezone)?;
    let zoned = Timestamp::from_second(occurred_at)?.to_zoned(tz);

    return Ok(zoned.date().to_string());
}
