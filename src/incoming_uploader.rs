use core::time;
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use crate::config::Config;

#[derive(Clone)]
pub enum SharingState {
    Off,
    Local,
    Internet,
}

pub struct IncomingUploader {
    sender: Sender<i32>,
}

impl IncomingUploader {

    pub fn new(config: Config) -> IncomingUploader {

        let (sx, rx): (
            Sender<i32>,
            Receiver<i32>,
        ) = mpsc::channel();

        thread::spawn(move || {
            let mut uploader_manager = UploaderManager::new(rx, config, SharingState::Off);
            uploader_manager.run();
        });

        return IncomingUploader {
            sender: sx,
        };
    }
}


struct UploaderManager {
    receiver: Receiver<i32>,
    config: Config,
    sharing_state: SharingState,
}

impl UploaderManager {

    pub fn new(receiver: Receiver<i32>, config: Config, sharing_state: SharingState) -> UploaderManager {

        return UploaderManager {
            receiver: receiver,
            config,
            sharing_state,
        }
    }

    pub fn run(&mut self) {
        println!("UploaderManager running");

        let mut ip_address = String::new();
        loop {

            match self.sharing_state {
                SharingState::Off => {
                    thread::sleep(time::Duration::from_secs(1));
                    continue;
                },
                SharingState::Local => {
                    ip_address = String::from("127.0.0.1");
                },
                SharingState::Internet => {
                    ip_address = String::from("0.0.0.0");
                },
            }

            ip_address.push_str(&format!(":{}", self.config.get_sharing_port()));
            println!("Waiting for incoming on {}", ip_address);

            let listener = TcpListener::bind(ip_address).unwrap();
            match listener.set_nonblocking(true) {
                Ok(_) => {},
                Err(e) => {
                    // TODO
                    todo!()
                },
            }
            

        }
    }

}