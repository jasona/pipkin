//! Versioned local persistence: preferences, drafts and demo-owned conversation data.
//!
//! One SQLite database (WAL, `synchronous=FULL`) with ONE background writer thread fed by a
//! bounded channel. A draft acknowledgment is delivered only after its transaction has
//! committed. Demo conversations live in `demo_`-prefixed tables so a real backend can replace
//! them without any migration into engine storage.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use pipkin_core::*;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

/// Writes queued beyond this are rejected as "busy" rather than blocking the UI thread.
const QUEUE_DEPTH: usize = 256;

pub const INJECTED_FAILURE: &str = "injected write failure";

const MIGRATIONS: &[&str] = &[
    "CREATE TABLE prefs (key TEXT PRIMARY KEY, value TEXT NOT NULL);
     CREATE TABLE drafts (
        conversation_id INTEGER PRIMARY KEY,
        text TEXT NOT NULL,
        rev INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
     );",
    "CREATE TABLE demo_conversations (
        id INTEGER PRIMARY KEY,
        project_id INTEGER NOT NULL,
        title TEXT NOT NULL,
        updated_at INTEGER NOT NULL
     );",
];

pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

pub type Ack = Box<dyn FnOnce(Result<(), String>) + Send>;

#[derive(Clone, Debug, PartialEq)]
pub struct DemoConversation {
    pub id: ConversationId,
    pub project: ProjectId,
    pub title: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredDraft {
    pub conversation: ConversationId,
    pub text: String,
    pub rev: u64,
}

#[derive(Debug)]
pub struct Loaded {
    pub prefs: Prefs,
    pub drafts: Vec<StoredDraft>,
    pub conversations: Vec<DemoConversation>,
}

enum Msg {
    Draft {
        conversation: ConversationId,
        text: String,
        rev: u64,
        ack: Ack,
    },
    Prefs(Prefs),
    Conversation(DemoConversation),
}

pub struct Storage {
    path: PathBuf,
    sender: Mutex<Option<SyncSender<Msg>>>,
    writer: Mutex<Option<JoinHandle<()>>>,
    fail_writes: Arc<AtomicBool>,
}

fn sql_err(e: rusqlite::Error) -> String {
    e.to_string()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn configure(conn: &Connection) -> Result<(), String> {
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(sql_err)?;
    // `journal_mode` returns a row, so it cannot go through `execute_batch`-style pragmas.
    conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))
        .map_err(sql_err)?;
    conn.execute_batch("PRAGMA synchronous=FULL;")
        .map_err(sql_err)
}

fn migrate(conn: &mut Connection) -> Result<(), String> {
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(sql_err)?;
    if version > SCHEMA_VERSION {
        return Err(format!(
            "database schema version {version} is newer than this build supports ({SCHEMA_VERSION})"
        ));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction().map_err(sql_err)?;
        tx.execute_batch(sql).map_err(sql_err)?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", i + 1))
            .map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
    }
    Ok(())
}

impl Storage {
    /// Open (creating and migrating if needed) the database at `path` and start the writer.
    pub fn open(path: &Path) -> Result<Storage, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let mut conn = Connection::open(path).map_err(sql_err)?;
        configure(&conn)?;
        migrate(&mut conn)?;

        let (tx, rx) = sync_channel::<Msg>(QUEUE_DEPTH);
        let fail_writes = Arc::new(AtomicBool::new(false));
        let fail = fail_writes.clone();
        let writer = std::thread::Builder::new()
            .name("pi-storage-writer".into())
            .spawn(move || {
                // Ends when every sender is dropped, after the queue has been drained.
                while let Ok(msg) = rx.recv() {
                    write(&mut conn, &fail, msg);
                }
            })
            .map_err(|e| format!("cannot start storage writer: {e}"))?;
        Ok(Storage {
            path: path.to_path_buf(),
            sender: Mutex::new(Some(tx)),
            writer: Mutex::new(Some(writer)),
            fail_writes,
        })
    }

    pub fn load_all(&self) -> Result<Loaded, String> {
        let conn = Connection::open(&self.path).map_err(sql_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(sql_err)?;
        let mut prefs = Prefs::default();
        let mut stmt = conn
            .prepare("SELECT key, value FROM prefs")
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(sql_err)?;
        for row in rows {
            let (key, value) = row.map_err(sql_err)?;
            if let Ok(v) = serde_json::from_str::<Value>(&value) {
                apply_pref(&mut prefs, &key, &v);
            }
        }
        let drafts = conn
            .prepare("SELECT conversation_id, text, rev FROM drafts ORDER BY conversation_id")
            .map_err(sql_err)?
            .query_map([], |r| {
                Ok(StoredDraft {
                    conversation: ConversationId(r.get::<_, i64>(0)? as u64),
                    text: r.get(1)?,
                    rev: r.get::<_, i64>(2)? as u64,
                })
            })
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        let conversations = conn
            .prepare("SELECT id, project_id, title, updated_at FROM demo_conversations ORDER BY id")
            .map_err(sql_err)?
            .query_map([], |r| {
                Ok(DemoConversation {
                    id: ConversationId(r.get::<_, i64>(0)? as u64),
                    project: ProjectId(r.get::<_, i64>(1)? as u64),
                    title: r.get(2)?,
                    updated_at: r.get(3)?,
                })
            })
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        Ok(Loaded {
            prefs,
            drafts,
            conversations,
        })
    }

    fn send(&self, msg: Msg) -> Result<(), Msg> {
        let guard = self.sender.lock().unwrap();
        match guard.as_ref() {
            None => Err(msg),
            Some(tx) => tx.try_send(msg).map_err(|e| match e {
                TrySendError::Full(m) | TrySendError::Disconnected(m) => m,
            }),
        }
    }

    /// Queue a draft write. `ack` runs on the writer thread only after the transaction has
    /// committed (or with the error). The caller keeps its own copy of the text either way.
    pub fn save_draft(
        &self,
        conversation: ConversationId,
        text: String,
        rev: u64,
        ack: impl FnOnce(Result<(), String>) + Send + 'static,
    ) {
        let msg = Msg::Draft {
            conversation,
            text,
            rev,
            ack: Box::new(ack),
        };
        if let Err(Msg::Draft { ack, .. }) = self.send(msg) {
            ack(Err("storage is busy or closed".into()));
        }
    }

    pub fn save_prefs(&self, prefs: &Prefs) {
        if self.send(Msg::Prefs(prefs.clone())).is_err() {
            log::warn!("preferences write dropped: storage busy or closed");
        }
    }

    pub fn save_conversation(&self, conversation: DemoConversation) {
        if self.send(Msg::Conversation(conversation)).is_err() {
            log::warn!("conversation write dropped: storage busy or closed");
        }
    }

    /// Make draft saves fail with `"injected write failure"` (developer control).
    pub fn set_write_failure(&self, fail: bool) {
        self.fail_writes.store(fail, Ordering::SeqCst);
    }

    pub fn write_failure(&self) -> bool {
        self.fail_writes.load(Ordering::SeqCst)
    }

    /// Drain the queue and join the writer. Safe to call more than once.
    pub fn shutdown(&self) {
        drop(self.sender.lock().unwrap().take());
        if let Some(handle) = self.writer.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn write(conn: &mut Connection, fail: &AtomicBool, msg: Msg) {
    match msg {
        Msg::Draft {
            conversation,
            text,
            rev,
            ack,
        } => {
            let result = if fail.load(Ordering::SeqCst) {
                Err(INJECTED_FAILURE.to_string())
            } else {
                write_draft(conn, conversation, &text, rev)
            };
            ack(result);
        }
        Msg::Prefs(prefs) => {
            if let Err(e) = write_prefs(conn, &prefs) {
                log::warn!("saving preferences failed: {e}");
            }
        }
        Msg::Conversation(c) => {
            if let Err(e) = write_conversation(conn, &c) {
                log::warn!("saving conversation failed: {e}");
            }
        }
    }
}

fn write_draft(
    conn: &mut Connection,
    id: ConversationId,
    text: &str,
    rev: u64,
) -> Result<(), String> {
    let tx = conn.transaction().map_err(sql_err)?;
    tx.execute(
        "INSERT INTO drafts (conversation_id, text, rev, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(conversation_id) DO UPDATE SET
            text = excluded.text, rev = excluded.rev, updated_at = excluded.updated_at",
        params![id.0 as i64, text, rev as i64, now_secs()],
    )
    .map_err(sql_err)?;
    tx.commit().map_err(sql_err)
}

fn write_conversation(conn: &mut Connection, c: &DemoConversation) -> Result<(), String> {
    let tx = conn.transaction().map_err(sql_err)?;
    tx.execute(
        "INSERT INTO demo_conversations (id, project_id, title, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            project_id = excluded.project_id, title = excluded.title, updated_at = excluded.updated_at",
        params![c.id.0 as i64, c.project.0 as i64, c.title, c.updated_at],
    )
    .map_err(sql_err)?;
    tx.commit().map_err(sql_err)
}

fn write_prefs(conn: &mut Connection, p: &Prefs) -> Result<(), String> {
    let entries: Vec<(&str, Value)> = vec![
        (
            "theme",
            json!(if p.theme == Theme::Light {
                "light"
            } else {
                "dark"
            }),
        ),
        (
            "text_size",
            json!(match p.text_size {
                TextSize::Small => "small",
                TextSize::Normal => "normal",
                TextSize::Large => "large",
            }),
        ),
        ("reduced_motion", json!(p.reduced_motion)),
        ("nav_width", json!(p.nav_width)),
        ("inspector_width", json!(p.inspector_width)),
        ("inspector_open", json!(p.inspector_open)),
        ("selected_project", json!(p.selected_project.map(|i| i.0))),
        (
            "selected_conversation",
            json!(p.selected_conversation.map(|i| i.0)),
        ),
        ("model", json!(p.model)),
    ];
    let tx = conn.transaction().map_err(sql_err)?;
    for (key, value) in entries {
        tx.execute(
            "INSERT INTO prefs (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value.to_string()],
        )
        .map_err(sql_err)?;
    }
    tx.commit().map_err(sql_err)
}

fn apply_pref(p: &mut Prefs, key: &str, v: &Value) {
    match key {
        "theme" => match v.as_str() {
            Some("light") => p.theme = Theme::Light,
            Some("dark") => p.theme = Theme::Dark,
            _ => {}
        },
        "text_size" => match v.as_str() {
            Some("small") => p.text_size = TextSize::Small,
            Some("normal") => p.text_size = TextSize::Normal,
            Some("large") => p.text_size = TextSize::Large,
            _ => {}
        },
        "reduced_motion" => p.reduced_motion = v.as_bool().unwrap_or(p.reduced_motion),
        "nav_width" => p.nav_width = v.as_f64().map_or(p.nav_width, |x| x as f32),
        "inspector_width" => p.inspector_width = v.as_f64().map_or(p.inspector_width, |x| x as f32),
        "inspector_open" => p.inspector_open = v.as_bool().unwrap_or(p.inspector_open),
        "selected_project" => p.selected_project = v.as_u64().map(ProjectId),
        "selected_conversation" => p.selected_conversation = v.as_u64().map(ConversationId),
        "model" => p.model = v.as_str().map(str::to_string),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    fn tmp_db() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        (dir, path)
    }

    fn save_and_wait(s: &Storage, conv: u64, text: &str, rev: u64) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        s.save_draft(ConversationId(conv), text.to_string(), rev, move |r| {
            tx.send(r).unwrap();
        });
        rx.recv_timeout(Duration::from_secs(10)).unwrap()
    }

    #[test]
    fn round_trip_prefs_drafts_and_conversations() {
        let (_dir, path) = tmp_db();
        let prefs = Prefs {
            theme: Theme::Light,
            text_size: TextSize::Large,
            reduced_motion: true,
            nav_width: 301.5,
            inspector_width: 512.0,
            inspector_open: false,
            selected_project: Some(ProjectId(2)),
            selected_conversation: Some(ConversationId(11)),
            model: Some("pi-opus".into()),
        };
        {
            let s = Storage::open(&path).unwrap();
            save_and_wait(
                &s,
                3,
                "h\u{e9}llo \u{1F9D1}\u{200D}\u{1F680} \u{645}\u{631}\u{62d}\u{628}\u{627}",
                7,
            )
            .unwrap();
            save_and_wait(&s, 3, "second", 8).unwrap();
            s.save_prefs(&prefs);
            s.save_conversation(DemoConversation {
                id: ConversationId(40),
                project: ProjectId(1),
                title: "New \u{2728} conversation".into(),
                updated_at: 123,
            });
            s.shutdown();
        }
        let s = Storage::open(&path).unwrap();
        let loaded = s.load_all().unwrap();
        assert_eq!(loaded.prefs, prefs);
        assert_eq!(
            loaded.drafts,
            vec![StoredDraft {
                conversation: ConversationId(3),
                text: "second".into(),
                rev: 8
            }]
        );
        assert_eq!(loaded.conversations.len(), 1);
        assert_eq!(loaded.conversations[0].title, "New \u{2728} conversation");
    }

    #[test]
    fn defaults_when_empty() {
        let (_dir, path) = tmp_db();
        let loaded = Storage::open(&path).unwrap().load_all().unwrap();
        assert_eq!(loaded.prefs, Prefs::default());
        assert!(loaded.drafts.is_empty() && loaded.conversations.is_empty());
    }

    #[test]
    fn injected_failure_reports_error_and_writes_nothing() {
        let (_dir, path) = tmp_db();
        let s = Storage::open(&path).unwrap();
        s.set_write_failure(true);
        assert!(s.write_failure());
        let err = save_and_wait(&s, 1, "unsaved text", 1).unwrap_err();
        assert_eq!(err, "injected write failure");
        assert!(s.load_all().unwrap().drafts.is_empty());
        s.set_write_failure(false);
        save_and_wait(&s, 1, "unsaved text", 2).unwrap();
        assert_eq!(s.load_all().unwrap().drafts[0].text, "unsaved text");
    }

    #[test]
    fn ack_is_delivered_only_after_commit() {
        let (_dir, path) = tmp_db();
        let s = Storage::open(&path).unwrap();
        let (tx, rx) = mpsc::channel();
        let reader_path = path.clone();
        s.save_draft(ConversationId(5), "durable".into(), 3, move |r| {
            // Inside the ack, an independent connection must already see the committed row.
            let conn = Connection::open(&reader_path).unwrap();
            let seen: Option<String> = conn
                .query_row(
                    "SELECT text FROM drafts WHERE conversation_id = 5 AND rev = 3",
                    [],
                    |r| r.get(0),
                )
                .ok();
            tx.send((r, seen)).unwrap();
        });
        let (result, seen) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(result, Ok(()));
        assert_eq!(seen.as_deref(), Some("durable"));
    }

    #[test]
    fn shutdown_drains_queue() {
        let (_dir, path) = tmp_db();
        let s = Storage::open(&path).unwrap();
        for rev in 1..=50u64 {
            s.save_draft(ConversationId(1), format!("rev {rev}"), rev, |_| {});
        }
        s.shutdown();
        s.save_draft(ConversationId(1), "late".into(), 99, |r| {
            assert!(r.is_err())
        });
        let drafts = Storage::open(&path).unwrap().load_all().unwrap().drafts;
        assert_eq!(drafts[0].rev, 50);
    }

    #[test]
    fn migrates_an_older_user_version() {
        let (_dir, path) = tmp_db();
        {
            // A "version 1" database: no demo_conversations table yet.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.execute_batch("PRAGMA user_version = 1").unwrap();
            conn.execute("INSERT INTO drafts VALUES (9, 'old draft', 4, 0)", [])
                .unwrap();
            conn.execute("INSERT INTO prefs VALUES ('theme', '\"light\"')", [])
                .unwrap();
        }
        let s = Storage::open(&path).unwrap();
        let loaded = s.load_all().unwrap();
        assert_eq!(loaded.drafts[0].text, "old draft");
        assert_eq!(loaded.prefs.theme, Theme::Light);
        assert!(loaded.conversations.is_empty());
        let conn = Connection::open(&path).unwrap();
        let v: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn refuses_a_newer_database() {
        let (_dir, path) = tmp_db();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(&format!("PRAGMA user_version = {}", SCHEMA_VERSION + 1))
                .unwrap();
        }
        let err = Storage::open(&path).err().expect("must refuse");
        assert!(err.contains("newer"), "{err}");
    }

    fn kill_text(conv: u64, rev: u64) -> String {
        format!("draft for conversation {conv}, revision {rev} \u{2014} \u{1F9EA} e\u{301}")
    }

    /// Child half of the kill test: only active when the parent sets the env switch.
    #[test]
    fn kill_child_writer() {
        let Some(dir) = std::env::var_os("PIPKIN_KILL_DIR") else {
            return;
        };
        let s = Storage::open(&PathBuf::from(dir).join("kill.sqlite3")).unwrap();
        for rev in 1..=100_000u64 {
            let conv = rev % 3 + 1;
            let (tx, rx) = mpsc::channel();
            s.save_draft(ConversationId(conv), kill_text(conv, rev), rev, move |r| {
                tx.send(r).unwrap();
            });
            rx.recv().unwrap().unwrap();
            println!("ACK {conv} {rev}");
        }
    }

    #[test]
    fn acknowledged_drafts_survive_sigkill() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "storage::tests::kill_child_writer",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("PIPKIN_KILL_DIR", dir.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut acked: Vec<(u64, u64)> = Vec::new();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        for line in lines {
            let line = line.unwrap();
            if let Some(rest) = line.strip_prefix("ACK ") {
                let mut parts = rest.split(' ').map(|p| p.parse::<u64>().unwrap());
                acked.push((parts.next().unwrap(), parts.next().unwrap()));
                if acked.len() >= 40 {
                    break;
                }
            }
        }
        child.kill().unwrap(); // SIGKILL on unix: no destructors, no flush.
        child.wait().unwrap();
        assert!(
            acked.len() >= 40,
            "child produced only {} acks",
            acked.len()
        );

        let s = Storage::open(&dir.path().join("kill.sqlite3")).unwrap();
        let drafts = s.load_all().unwrap().drafts;
        for conv in 1..=3u64 {
            let max_acked = acked
                .iter()
                .filter(|a| a.0 == conv)
                .map(|a| a.1)
                .max()
                .unwrap();
            let stored = drafts
                .iter()
                .find(|d| d.conversation == ConversationId(conv))
                .unwrap();
            assert!(
                stored.rev >= max_acked,
                "conversation {conv}: lost acknowledged rev {max_acked}"
            );
            assert_eq!(
                stored.text,
                kill_text(conv, stored.rev),
                "conversation {conv} corrupted"
            );
        }
    }
}
