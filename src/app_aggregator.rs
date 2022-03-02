use std::collections::{VecDeque, HashMap};
use std::hash::Hash;
use std::sync::{mpsc, RwLock, Arc};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use crate::transmitic_core::{SingleDownloadState, SingleUploadState};


struct DownloadStatus {

}

struct DownloadUpdateMessage {
    pub owner: String,
}

// TODO combine them all into 1 struct?
pub struct InvalidFileMessage {
    pub nickname: String,
    pub invalid_path: String,
    pub download_queue: VecDeque<String>,
}

pub struct InProgressMessage {
    pub nickname: String,
    pub path: Option<String>,
    pub percent: u64,
    pub download_queue: VecDeque<String>, 
    pub path_local_disk: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CompletedMessage {
    pub nickname: String,
    pub path: String,
    pub download_queue: VecDeque<String>,
    pub path_local_disk: String,
}

pub struct OfflineMessage {
    pub nickname: String,
    pub download_queue: VecDeque<String>,
}

pub enum AppAggMessage {
    StringLog(String),
    InvalidFile(InvalidFileMessage),
    InProgress(InProgressMessage),
    Completed(CompletedMessage),
    Offline(OfflineMessage),
    UploadStateChange(SingleUploadState),
}

// TODO clean up file

pub struct AppAggregator {
    sender: Option<Sender<AppAggMessage>>,
}

impl AppAggregator {

    pub fn new() -> AppAggregator {

        return AppAggregator {
            sender: None,
        }

    }

    pub fn start(&self, downlaod_state: Arc<RwLock<HashMap<String, SingleDownloadState>>>, upload_state: Arc<RwLock<HashMap<String, SingleUploadState>>>) -> Sender<AppAggMessage>{
        let (sender, receiver): (Sender<AppAggMessage>, Receiver<AppAggMessage>) = mpsc::channel();

        thread::spawn(move || {
            app_loop(receiver, downlaod_state, upload_state);
        });

        return sender;
    }

}

fn app_loop(receiver: Receiver<AppAggMessage>, download_state: Arc<RwLock<HashMap<String, SingleDownloadState>>>, upload_state: Arc<RwLock<HashMap<String, SingleUploadState>>>) {
    loop {
        let msg = match receiver.recv() {
            Ok(msg) => msg,
            Err(e) => todo!(),
        };

        match msg {
            // TODO clean up. Struct for download_state instead
            AppAggMessage::StringLog(s) => println!("[LOG] {}", s),
            AppAggMessage::InvalidFile(f) => {
                let mut l = download_state.write().unwrap();
                match l.get_mut(&f.nickname) {
                    Some(h) => {
                        h.invalid_downloads.push(f.invalid_path);
                        h.download_queue = f.download_queue;
                        h.is_online = true;
                    },
                    None => {
                        let mut s = SingleDownloadState::new();
                        s.invalid_downloads.push(f.invalid_path);
                        s.download_queue = f.download_queue;
                        l.insert(f.nickname, s);
                    },
                }
            },
            AppAggMessage::InProgress(f) => {
                let mut l = download_state.write().unwrap();
                match l.get_mut(&f.nickname) {
                    Some(h) => {
                        h.active_download_path = f.path;
                        h.active_download_percent = f.percent;
                        h.active_download_local_path = f.path_local_disk;
                        h.download_queue = f.download_queue;
                        h.is_online = true;
                    },
                    None => {
                        let mut s = SingleDownloadState::new();
                        s.active_download_path = f.path;
                        s.active_download_local_path = f.path_local_disk;
                        s.active_download_percent = f.percent;
                        s.download_queue = f.download_queue;
                        l.insert(f.nickname, s);
                    },
                }
            },
            AppAggMessage::Completed(f) => {
                let mut l = download_state.write().unwrap();
                match l.get_mut(&f.nickname) {
                    Some(h) => {
                        h.completed_downloads.push(f.clone());
                        h.download_queue = f.download_queue;
                        h.active_download_path = None;
                        h.active_download_local_path = Some(f.path_local_disk);
                        h.is_online = true;
                    },
                    None => {
                        let mut s = SingleDownloadState::new();
                        s.completed_downloads.push(f.clone());
                        s.download_queue = f.download_queue;
                        s.active_download_path = None;
                        l.insert(f.nickname, s);
                    },
                }
            },
            AppAggMessage::Offline(f) => {
                let mut l = download_state.write().unwrap();
                match l.get_mut(&f.nickname) {
                    Some(h) => {
                        h.is_online = false;
                        h.download_queue = f.download_queue;
                    },
                    None => {
                        let mut s = SingleDownloadState::new();
                        s.is_online = false;
                        s.download_queue = f.download_queue;
                        l.insert(f.nickname, s);
                    },
                }
            },
            AppAggMessage::UploadStateChange(f) => {
                let mut l = upload_state.write().unwrap();
                l.insert(f.nickname.clone(), f);
            },
        }
    }
}