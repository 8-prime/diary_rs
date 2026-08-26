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

pub struct Entry {
    pub id: i64,
    pub diary_id: i64,
    pub local_date: String,
    pub occurred_at: i64,
    pub created_at: i64,
    pub text: Option<String>,
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

    // Spawns a task or needs to be run in a task. performs
    // bg image gc
    pub async fn run_blob_gc(&self) -> () {}

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
    pub fn get_diary(&self) {}
    // Creates a new diary entry and generates required data
    // (keys and id)
    pub fn create_diary(&self) {}
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
