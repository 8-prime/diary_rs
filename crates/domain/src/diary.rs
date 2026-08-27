use std::collections::HashMap;

use sqlx::SqlitePool;

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

pub struct DiaryService {
    write: SqlitePool,
    read: SqlitePool,
    // blob storatge access
}

impl DiaryService {
    pub fn new(write: SqlitePool, read: SqlitePool) -> DiaryService {
        return DiaryService { write, read };
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

        // Sorted the way the page renders: newest day first, and within a day
        // the entries in the order they happened.
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
    pub fn create_diary(&self, diary_view: DiaryView) {}
    // Updates title and description of diary
    pub fn update_diary(&self) {}
    // delete diary and entries
    pub fn delete_diary(&self) {}
    // rerolls the public access link id
    pub fn reroll_diary_keys(&self) {}
    pub fn create_entry(&self) {}
    // update diary text
    pub fn update_entry(&self) {}
    pub fn delete_entry(&self) {}
    // stores the blob but does not store in db yet (done by create entry)
    pub fn create_photo(&self) {}
    // removes only the db entry but not the blob?!
    pub fn delete_photo(&self) {}
}
