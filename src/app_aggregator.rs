use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;


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
    pub path: String,
    pub percent: u64,
    pub download_queue: VecDeque<String>, 
}

pub struct CompletedMessage {
    pub nickname: String,
    pub path: String,
    pub download_queue: VecDeque<String>,
}

pub enum AppAggMessage {
    StringLog(String),
    InvalidFile(InvalidFileMessage),
    InProgress(InProgressMessage),
    Completed(CompletedMessage),
}


pub struct AppAggregator {
    sender: Option<Sender<AppAggMessage>>,
}

impl AppAggregator {

    pub fn new() -> AppAggregator {

        return AppAggregator {
            sender: None,
        }

    }

    pub fn start(&self) -> Sender<AppAggMessage>{
        let (sender, receiver): (Sender<AppAggMessage>, Receiver<AppAggMessage>) = mpsc::channel();

        thread::spawn(move || {
            app_loop(receiver);
        });

        return sender;
    }

}

fn app_loop(receiver: Receiver<AppAggMessage>) {
    loop {
        let msg = match receiver.recv() {
            Ok(msg) => msg,
            Err(e) => todo!(),
        };

        match msg {
            AppAggMessage::StringLog(s) => println!("[LOG] {}", s),
            AppAggMessage::InvalidFile(_) => todo!(),
            AppAggMessage::InProgress(_) => todo!(),
            AppAggMessage::Completed(_) => todo!(),
        }
    }
}