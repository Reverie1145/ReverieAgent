//! `session_attachments` repository.

use reverie_core::domain::SessionAttachment;
use reverie_core::time;
use crate::db::SqliteRow;

use crate::db::{i, os, s};
use crate::error::StoreResult;
use crate::Store;

const COLUMNS: &str = "id, session_id, file_name, file_path, mime_type, file_size_bytes, base64_thumbnail, \
     extracted_text_preview, created_at";

fn map_row(row: &SqliteRow) -> SessionAttachment {
    SessionAttachment {
        id: s(row, "id"),
        session_id: s(row, "session_id"),
        file_name: s(row, "file_name"),
        file_path: s(row, "file_path"),
        mime_type: s(row, "mime_type"),
        file_size_bytes: i(row, "file_size_bytes"),
        base64_thumbnail: os(row, "base64_thumbnail"),
        extracted_text_preview: os(row, "extracted_text_preview"),
        created_at: time::from_db(&s(row, "created_at")),
    }
}

impl Store {
    pub async fn insert_attachment(&self, attachment: &SessionAttachment) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO session_attachments (
                id, session_id, file_name, file_path, mime_type, file_size_bytes,
                base64_thumbnail, extracted_text_preview, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                session_id = excluded.session_id,
                file_name = excluded.file_name,
                file_path = excluded.file_path,
                mime_type = excluded.mime_type,
                file_size_bytes = excluded.file_size_bytes,
                base64_thumbnail = excluded.base64_thumbnail,
                extracted_text_preview = excluded.extracted_text_preview",
        )
        .bind(&attachment.id)
        .bind(&attachment.session_id)
        .bind(&attachment.file_name)
        .bind(&attachment.file_path)
        .bind(&attachment.mime_type)
        .bind(attachment.file_size_bytes)
        .bind(&attachment.base64_thumbnail)
        .bind(&attachment.extracted_text_preview)
        .bind(time::to_db(attachment.created_at))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Insert many staged attachments in one transaction.
    pub async fn insert_attachments(&self, attachments: &[SessionAttachment]) -> StoreResult<()> {
        let mut tx = self.pool().begin().await?;
        for attachment in attachments {
            sqlx::query(
                "INSERT INTO session_attachments (
                    id, session_id, file_name, file_path, mime_type, file_size_bytes,
                    base64_thumbnail, extracted_text_preview, created_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET session_id = excluded.session_id",
            )
            .bind(&attachment.id)
            .bind(&attachment.session_id)
            .bind(&attachment.file_name)
            .bind(&attachment.file_path)
            .bind(&attachment.mime_type)
            .bind(attachment.file_size_bytes)
            .bind(&attachment.base64_thumbnail)
            .bind(&attachment.extracted_text_preview)
            .bind(time::to_db(attachment.created_at))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_attachments(&self, session_id: &str) -> StoreResult<Vec<SessionAttachment>> {
        let sql = format!("SELECT {COLUMNS} FROM session_attachments WHERE session_id = ? ORDER BY created_at ASC");
        let rows = sqlx::query(&sql).bind(session_id).fetch_all(self.pool()).await?;
        Ok(rows.iter().map(map_row).collect())
    }

    pub async fn get_attachment(&self, id: &str) -> StoreResult<Option<SessionAttachment>> {
        let sql = format!("SELECT {COLUMNS} FROM session_attachments WHERE id = ?");
        let row = sqlx::query(&sql).bind(id).fetch_optional(self.pool()).await?;
        Ok(row.as_ref().map(map_row))
    }

    /// Load a set of attachments by id, preserving the caller's order.
    pub async fn get_attachments(&self, ids: &[String]) -> StoreResult<Vec<SessionAttachment>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!("SELECT {COLUMNS} FROM session_attachments WHERE id IN ({placeholders})");
        let mut query = sqlx::query(&sql);
        for id in ids {
            query = query.bind(id);
        }
        let rows = query.fetch_all(self.pool()).await?;
        let loaded: Vec<SessionAttachment> = rows.iter().map(map_row).collect();
        let mut ordered = Vec::with_capacity(loaded.len());
        for id in ids {
            if let Some(found) = loaded.iter().find(|a| &a.id == id) {
                ordered.push(found.clone());
            }
        }
        Ok(ordered)
    }

    /// Bind previously staged attachments to a real session.
    pub async fn bind_attachments(&self, ids: &[String], session_id: &str) -> StoreResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!("UPDATE session_attachments SET session_id = ? WHERE id IN ({placeholders})");
        let mut query = sqlx::query(&sql).bind(session_id);
        for id in ids {
            query = query.bind(id);
        }
        query.execute(self.pool()).await?;
        Ok(())
    }

    pub async fn delete_attachment(&self, id: &str) -> StoreResult<()> {
        sqlx::query("DELETE FROM session_attachments WHERE id = ?").bind(id).execute(self.pool()).await?;
        Ok(())
    }

    pub async fn count_attachments(&self, session_id: &str) -> StoreResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM session_attachments WHERE session_id = ?")
            .bind(session_id)
            .fetch_one(self.pool())
            .await?;
        Ok(i(&row, "c"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::domain::Session;

    async fn seeded() -> (Store, Session) {
        let store = Store::connect_in_memory().await.unwrap();
        let session = Session::create("t", "F:/ws", "o", "auto", true);
        store.insert_session(&session).await.unwrap();
        (store, session)
    }

    #[tokio::test]
    async fn insert_and_list() {
        let (store, session) = seeded().await;
        let attachment = SessionAttachment::create(
            &session.id,
            "diagram.png",
            "F:/ws/.reverie/attachments/diagram.png",
            "image/png",
            4096,
        )
        .with_thumbnail("AAAA");
        store.insert_attachment(&attachment).await.unwrap();

        let list = store.list_attachments(&session.id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].base64_thumbnail.as_deref(), Some("AAAA"));
        assert!(list[0].is_image());
    }

    #[tokio::test]
    async fn batch_lookup_preserves_order() {
        let (store, session) = seeded().await;
        let mut ids = Vec::new();
        for name in ["a.txt", "b.txt", "c.txt"] {
            let a = SessionAttachment::create(&session.id, name, "/tmp/x", "text/plain", 10);
            ids.push(a.id.clone());
            store.insert_attachment(&a).await.unwrap();
        }
        let reversed: Vec<String> = ids.iter().rev().cloned().collect();
        let loaded = store.get_attachments(&reversed).await.unwrap();
        assert_eq!(loaded.iter().map(|a| a.id.clone()).collect::<Vec<_>>(), reversed);
    }

    #[tokio::test]
    async fn binding_reassigns_rows_to_another_session() {
        let (store, session) = seeded().await;
        let other = Session::create("t2", "F:/ws", "o", "auto", true);
        store.insert_session(&other).await.unwrap();

        let attachment = SessionAttachment::create(&session.id, "x.md", "/tmp/x.md", "text/markdown", 12);
        store.insert_attachment(&attachment).await.unwrap();
        assert_eq!(store.count_attachments(&session.id).await.unwrap(), 1);

        store.bind_attachments(&[attachment.id.clone()], &other.id).await.unwrap();
        assert_eq!(store.count_attachments(&other.id).await.unwrap(), 1);
        assert_eq!(store.count_attachments(&session.id).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn empty_lookup_is_cheap() {
        let (store, _) = seeded().await;
        assert!(store.get_attachments(&[]).await.unwrap().is_empty());
        store.bind_attachments(&[], "s").await.unwrap();
    }
}
