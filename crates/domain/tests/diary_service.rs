// Claude written

use std::collections::HashSet;

use domain::diary::{DiaryService, NewDiary, NewEntry, NewImage};
use domain::images::ImageService;
use jiff::Timestamp;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::new(width, height))
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    out
}

fn ts(s: &str) -> i64 {
    s.parse::<Timestamp>().unwrap().as_second()
}

#[test]
fn wiring() {
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();

    rt.block_on(async {
        let opts = SqliteConnectOptions::new()
            .filename(dir.path().join("test.db"))
            .create_if_missing(true)
            .foreign_keys(true);

        let write = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts.clone())
            .await
            .unwrap();
        let read = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(opts)
            .await
            .unwrap();

        sqlx::migrate!("./migrations").run(&write).await.unwrap();

        std::fs::create_dir_all(dir.path().join("images")).unwrap();
        let images = ImageService::new(dir.path().join("images"));
        let svc = DiaryService::new(write.clone(), read.clone(), images.clone());

        // ---- photo -> diary -> entries -------------------------------------
        let photo = images.upload_image(&jpeg(1000, 500)).await.unwrap();
        assert_eq!((photo.width, photo.height), (1000, 500));

        let diary = svc
            .create_diary(NewDiary {
                title: "Mochi".into(),
                description: Some("week one".into()),
                timezone: "Europe/Berlin".into(),
            })
            .await
            .unwrap();
        assert_eq!(diary.share_token.len(), 43, "{}", diary.share_token);

        // 23:30Z is already the next calendar day in Berlin.
        let late = svc
            .create_entry(
                diary.id,
                NewEntry {
                    occurred_at: ts("2026-01-01T23:30:00Z"),
                    text: Some("late night".into()),
                    images: vec![NewImage {
                        hash: photo.hash.clone(),
                        width: photo.width,
                        height: photo.height,
                        alt: Some("a cat".into()),
                    }],
                },
            )
            .await
            .unwrap();
        assert_eq!(late.entry.local_date, "2026-01-02");
        assert_eq!(late.images.len(), 1);
        assert_eq!((late.images[0].width, late.images[0].height), (1000, 500));
        assert_eq!(late.images[0].position, 0);

        let early = svc
            .create_entry(
                diary.id,
                NewEntry {
                    occurred_at: ts("2026-01-01T09:00:00Z"),
                    text: None,
                    images: vec![],
                },
            )
            .await
            .unwrap();
        assert_eq!(early.entry.local_date, "2026-01-01");

        // ---- read back through the public path -----------------------------
        let view = svc.get_shared_diary(&diary.share_token).await.unwrap();
        let days: Vec<&str> = view.days.iter().map(|d| d.local_date.as_str()).collect();
        assert_eq!(days, ["2026-01-02", "2026-01-01"], "newest day first");
        assert_eq!(view.days[0].entries[0].images.len(), 1);

        // ---- unknown / hostile hashes --------------------------------------
        for bad in ["../../etc", &"0".repeat(64), "not-a-hash"] {
            let err = svc
                .create_entry(
                    diary.id,
                    NewEntry {
                        occurred_at: ts("2026-01-03T09:00:00Z"),
                        text: None,
                        images: vec![NewImage {
                            hash: bad.to_string(),
                            width: 100,
                            height: 100,
                            alt: None,
                        }],
                    },
                )
                .await;
            assert!(matches!(err, Err(domain::Error::NotFound)), "{bad}");
        }
        // ...and nothing was written by the failed attempts.
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM entry")
            .fetch_one(&read)
            .await
            .unwrap();
        assert_eq!(n, 2);

        // ---- reroll invalidates the old link -------------------------------
        let fresh = svc.reroll_diary_keys(diary.id).await.unwrap();
        assert!(matches!(
            svc.get_shared_diary(&diary.share_token).await,
            Err(domain::Error::NotFound)
        ));
        svc.get_shared_diary(&fresh).await.unwrap();

        // ---- gc leaves in-flight blobs alone -------------------------------
        let referenced = svc.referenced_hashes().await.unwrap();
        assert_eq!(referenced, HashSet::from([photo.hash.clone()]));
        assert_eq!(images.run_blob_gc(&referenced).unwrap(), 0, "blob is fresh");

        let orphan = dir.path().join("images").join("f".repeat(64));
        std::fs::create_dir_all(&orphan).unwrap();
        assert_eq!(images.run_blob_gc(&referenced).unwrap(), 0, "orphan is fresh");

        // ---- deletes --------------------------------------------------------
        svc.delete_photo(late.images[0].id).await.unwrap();
        assert!(matches!(
            svc.delete_photo(late.images[0].id).await,
            Err(domain::Error::NotFound)
        ));
        assert!(
            dir.path().join("images").join(&photo.hash).exists(),
            "blob must outlive the row"
        );

        svc.delete_entry(early.entry.id).await.unwrap();
        svc.delete_diary(diary.id).await.unwrap();

        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM entry")
            .fetch_one(&read)
            .await
            .unwrap();
        assert_eq!(n, 0, "cascade should have taken the entries");

        // Nothing references the blob now, but the grace period still holds.
        assert!(svc.referenced_hashes().await.unwrap().is_empty());

        assert!(matches!(
            svc.get_diary(diary.id).await,
            Err(domain::Error::NotFound)
        ));
    });
}

#[test]
fn reads_wall_clock_times_in_the_diary_timezone() {
    use domain::diary::timestamp_from_local;

    // Winter: Berlin is UTC+1, so 00:30 local is 23:30 the previous day UTC.
    assert_eq!(
        timestamp_from_local("Europe/Berlin", "2026-01-02T00:30").unwrap(),
        ts("2026-01-01T23:30:00Z")
    );
    // Summer: UTC+2.
    assert_eq!(
        timestamp_from_local("Europe/Berlin", "2026-07-01T12:00:00").unwrap(),
        ts("2026-07-01T10:00:00Z")
    );
    assert!(timestamp_from_local("Europe/Berlin", "nonsense").is_err());
    assert!(timestamp_from_local("Not/AZone", "2026-01-02T00:30").is_err());
}
