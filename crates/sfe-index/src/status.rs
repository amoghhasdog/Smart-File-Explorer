//! Progress snapshot published to the UI.

use std::sync::Mutex;

use serde::Serialize;
use sfe_core::{IndexState, RootId};
use sfe_db::{Database, files, jobs};
use tokio::sync::watch;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    /// Live, non-directory files known to the index.
    pub total_files: u64,
    pub discovered: u64,
    pub metadata: u64,
    pub text: u64,
    pub embedded: u64,
    pub failed: u64,
    pub pending_jobs: u64,
    /// Roots currently being walked.
    pub scanning: Vec<RootId>,
    pub paused: bool,
    pub last_error: Option<String>,
}

impl IndexStatus {
    /// `true` while there is outstanding work.
    pub fn busy(&self) -> bool {
        self.pending_jobs > 0 || !self.scanning.is_empty()
    }
}

pub(crate) struct StatusHub {
    tx: watch::Sender<IndexStatus>,
    last_error: Mutex<Option<String>>,
}

impl StatusHub {
    pub fn new(db: &Database) -> Self {
        let hub = Self {
            tx: watch::channel(IndexStatus::default()).0,
            last_error: Mutex::new(None),
        };
        hub.refresh(db, false, &[]);
        hub
    }

    pub fn subscribe(&self) -> watch::Receiver<IndexStatus> {
        self.tx.subscribe()
    }

    pub fn current(&self) -> IndexStatus {
        self.tx.borrow().clone()
    }

    pub fn set_error(&self, e: String) {
        *self.last_error.lock().unwrap() = Some(e);
    }

    pub fn refresh(&self, db: &Database, paused: bool, scanning: &[RootId]) {
        let counted = {
            let conn = db.writer();
            files::count_by_state(&conn).and_then(|s| jobs::count_pending(&conn).map(|p| (s, p)))
        };
        let mut st = IndexStatus {
            paused,
            scanning: scanning.to_vec(),
            last_error: self.last_error.lock().unwrap().clone(),
            ..Default::default()
        };
        match counted {
            Ok((states, pending)) => {
                for (state, n) in states {
                    st.total_files += n;
                    match state {
                        IndexState::Discovered => st.discovered = n,
                        IndexState::Metadata => st.metadata = n,
                        IndexState::Text => st.text = n,
                        IndexState::Embedded => st.embedded = n,
                        IndexState::Failed => st.failed = n,
                    }
                }
                st.pending_jobs = pending;
            }
            Err(e) => st.last_error = Some(e.to_string()),
        }
        self.tx.send_if_modified(|cur| {
            if *cur != st {
                *cur = st;
                true
            } else {
                false
            }
        });
    }
}
