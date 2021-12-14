use core::time;
use std::net::{TcpListener, TcpStream};
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

enum MessageUploadManager {

}

enum MessageSingleUploader {

}

pub struct IncomingUploader {
    sender: Sender<MessageUploadManager>,
}

impl IncomingUploader {

    pub fn new(config: Config) -> IncomingUploader {

        let (sx, rx): (
            Sender<MessageUploadManager>,
            Receiver<MessageUploadManager>,
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
    receiver: Receiver<MessageUploadManager>,
    config: Config,
    sharing_state: SharingState,
    // TODO once a SingleUplaoder thread is dead, is the reciever dead, and thus can be removed from
    // this vec on next interation? If not, SingleUploader needs to send a message back around.
    // 1. Map usize to Senders. SingleUploader gets its ID. At shutdown send to AppAggreator to pop it
    //  When adding a new one loop through usize until you find available int.
    single_uploaders: Vec<Sender<MessageSingleUploader>>,
}

impl UploaderManager {

    pub fn new(receiver: Receiver<MessageUploadManager>, config: Config, sharing_state: SharingState) -> UploaderManager {

        let single_uploaders = Vec::new();

        return UploaderManager {
            receiver: receiver,
            config,
            sharing_state,
            single_uploaders,
        }
    }

    pub fn run(&mut self) {
        println!("UploaderManager running");

        let mut ip_address;
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

            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(stream) => {
                        let (sx, rx): (
                            Sender<MessageSingleUploader>,
                            Receiver<MessageSingleUploader>,
                        ) = mpsc::channel();
                        self.single_uploaders.push(sx);
                        let single_config = self.config.clone();
                        thread::spawn(move || {
                            let mut single_uploader = SingleUploader::new(rx, single_config, stream);
                            single_uploader.run();
                        });

                    },
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(time::Duration::from_secs(1));
                        continue;
                    }
                    Err(e) => {
                        // TODO
                        println!("ERROR: Failed initial client connection");
                        println!("{:?}", e);
                        continue;
                    }
                };


            }
            

        }
    }

}


struct SingleUploader {
    receiver: Receiver<MessageSingleUploader>,
    config: Config,
    stream: TcpStream,
    nickname: String,
}

impl SingleUploader {

    pub fn new(receiver: Receiver<MessageSingleUploader>, config: Config, stream: TcpStream) -> SingleUploader {

        let nickname =  String::new();
        return SingleUploader {
            receiver,
            config,
            stream,
            nickname,
        }
    }

    pub fn run(&mut self) {
        
    }
}