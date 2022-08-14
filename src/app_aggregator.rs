use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, Mutex, RwLock};

use std::thread;

use crate::logger::{LogLevel, Logger};
use crate::transmitic_core::{SingleDownloadState, SingleUploadState};

// TODO combine them all into 1 struct?
pub struct InvalidFileMessage {
    pub nickname: String,
    pub active_path: Option<String>,
    pub invalid_path: String,
    pub download_queue: VecDeque<String>,
}

pub struct InProgressMessage {
    pub nickname: String,
    pub path: Option<String>,
    pub percent: u64,
    pub download_queue: VecDeque<String>,
    pub path_local_disk: Option<String>,
    pub size_string: String,
}

#[derive(Clone, Debug)]
pub struct CompletedMessage {
    pub nickname: String,
    pub path: String,
    pub download_queue: VecDeque<String>,
    pub path_local_disk: String,
    pub size_string: String,
}

pub struct OfflineMessage {
    pub nickname: String,
    pub download_queue: VecDeque<String>,
}

pub enum AppAggMessage {
    LogDebug(String),
    LogInfo(String),
    LogWarning(String),
    LogError(String),
    LogCritical(String),
    SetLogLevel(LogLevel),
    LogToFileState(bool),
    InvalidFile(InvalidFileMessage),
    InProgress(InProgressMessage),
    Completed(CompletedMessage),
    Offline(OfflineMessage),
    UploadStateChange(SingleUploadState),
    UploadDisconnected(String),
    AppFailedKill(String),
}

pub fn run_app_loop(
    downlaod_state: Arc<RwLock<HashMap<String, SingleDownloadState>>>,
    upload_state: Arc<RwLock<HashMap<String, SingleUploadState>>>,
    logger: Arc<Mutex<Logger>>,
) -> Sender<AppAggMessage> {
    let (sender, receiver): (Sender<AppAggMessage>, Receiver<AppAggMessage>) = mpsc::channel();

    thread::spawn(move || {
        app_loop(receiver, downlaod_state, upload_state, logger);
    });

    sender
}

// TODO use attribute? i32
pub enum ExitCodes {
    AppLoopRecFailed = 2,
    AppFailedKill = 3,
}

#[allow(clippy::field_reassign_with_default)]
fn app_loop(
    receiver: Receiver<AppAggMessage>,
    download_state: Arc<RwLock<HashMap<String, SingleDownloadState>>>,
    upload_state: Arc<RwLock<HashMap<String, SingleUploadState>>>,
    logger: Arc<Mutex<Logger>>,
) {
    loop {
        let msg = match receiver.recv() {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("AppLoopRec failed. {}", e);
                std::process::exit(ExitCodes::AppLoopRecFailed as i32)
            }
        };

        match msg {
            // TODO clean up. Struct for download_state instead
            // TODO improve variable names
            AppAggMessage::InvalidFile(f) => {
                let mut l = download_state.write().unwrap();
                log_message(
                    &logger,
                    format!(
                        "Invalid file from '{}' - '{}'",
                        &f.nickname, &f.invalid_path
                    ),
                    LogLevel::Warning,
                );
                match l.get_mut(&f.nickname) {
                    Some(h) => {
                        h.active_download_path = f.active_path;
                        h.invalid_downloads.push(f.invalid_path);
                        h.download_queue = f.download_queue;
                        h.is_online = true;
                    }
                    None => {
                        let mut s = SingleDownloadState::default();
                        s.active_download_path = f.active_path;
                        s.invalid_downloads.push(f.invalid_path);
                        s.download_queue = f.download_queue;
                        l.insert(f.nickname, s);
                    }
                }
            }
            AppAggMessage::InProgress(f) => {
                let mut l = download_state.write().unwrap();
                match l.get_mut(&f.nickname) {
                    Some(h) => {
                        h.active_download_path = f.path;
                        h.active_download_percent = f.percent;
                        h.active_download_local_path = f.path_local_disk;
                        h.download_queue = f.download_queue;
                        h.is_online = true;
                        h.active_download_size = f.size_string;
                    }
                    None => {
                        let mut s = SingleDownloadState::default();
                        s.active_download_path = f.path;
                        s.active_download_local_path = f.path_local_disk;
                        s.active_download_percent = f.percent;
                        s.download_queue = f.download_queue;
                        s.active_download_size = f.size_string;
                        l.insert(f.nickname, s);
                    }
                }
            }
            AppAggMessage::Completed(f) => {
                let mut l = download_state.write().unwrap();
                log_message(
                    &logger,
                    format!(
                        "Download completed '{}' - '{}'",
                        &f.nickname, &f.path_local_disk
                    ),
                    LogLevel::Info,
                );
                match l.get_mut(&f.nickname) {
                    Some(h) => {
                        h.completed_downloads.push(f.clone());
                        h.download_queue = f.download_queue;
                        h.active_download_path = None;
                        h.active_download_local_path = Some(f.path_local_disk);
                        h.is_online = true;
                    }
                    None => {
                        let mut s = SingleDownloadState::default();
                        s.completed_downloads.push(f.clone());
                        s.download_queue = f.download_queue;
                        s.active_download_path = None;
                        l.insert(f.nickname, s);
                    }
                }
            }
            AppAggMessage::Offline(f) => {
                let mut l = download_state.write().unwrap();
                match l.get_mut(&f.nickname) {
                    Some(h) => {
                        h.is_online = false;
                        h.download_queue = f.download_queue;
                    }
                    None => {
                        let mut s = SingleDownloadState::default();
                        s.is_online = false;
                        s.download_queue = f.download_queue;
                        l.insert(f.nickname, s);
                    }
                }
            }
            AppAggMessage::UploadStateChange(f) => {
                let mut l = upload_state.write().unwrap();
                l.insert(f.nickname.clone(), f);
            }
            AppAggMessage::UploadDisconnected(nickname) => {
                let mut l = upload_state.write().unwrap();
                log_message(
                    &logger,
                    format!("Upload disconnected '{}'", &nickname),
                    LogLevel::Debug,
                );
                // There was Some existing download in progress
                if let Some(state) = l.get(&nickname) {
                    let mut state = state.clone();
                    state.is_online = false;
                    l.insert(nickname, state);
                }
            }
            AppAggMessage::AppFailedKill(s) => {
                eprintln!("AppFailedKill. {}", s);
                std::process::exit(ExitCodes::AppFailedKill as i32);
            }
            AppAggMessage::LogDebug(message) => log_message(&logger, message, LogLevel::Debug),
            AppAggMessage::LogInfo(message) => log_message(&logger, message, LogLevel::Info),
            AppAggMessage::LogWarning(message) => log_message(&logger, message, LogLevel::Warning),
            AppAggMessage::LogError(message) => log_message(&logger, message, LogLevel::Error),
            AppAggMessage::LogCritical(message) => {
                log_message(&logger, message, LogLevel::Critical)
            }
            AppAggMessage::SetLogLevel(log_level) => {
                let mut logger_guard = logger.lock().unwrap_or_else(|err| err.into_inner());
                logger_guard.set_log_level(log_level);
            }
            AppAggMessage::LogToFileState(state) => {
                let mut logger_guard = logger.lock().unwrap_or_else(|err| err.into_inner());
                logger_guard.set_is_file_logging(state);
            }
        }
    }
}

fn log_message(logger: &Arc<Mutex<Logger>>, message: String, log_level: LogLevel) {
    let mut logger_guard = logger.lock().unwrap_or_else(|err| err.into_inner());
    logger_guard.log_message(log_level, message);
}
