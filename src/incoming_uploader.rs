use core::time;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, Shutdown};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use serde::__private::de::TagContentOtherField;

use crate::config::{Config, SharedUser};
use crate::core_consts::{TRAN_MAGIC_NUMBER, TRAN_API_MAJOR, TRAN_API_MINOR, CONN_ESTABLISH_REQUEST, CONN_ESTABLISH_ACCEPT};
use crate::transmitic_stream::TransmiticStream;

#[derive(Clone)]
pub enum SharingState {
    Off,
    Local,
    Internet,
}

enum MessageUploadManager {

}



pub struct IncomingUploader {
    sender: Sender<MessageUploadManager>,
}

impl IncomingUploader {

    pub fn new(config: Config) -> IncomingUploader {
        println!("Starting incoming uploader");
        let (sx, rx): (
            Sender<MessageUploadManager>,
            Receiver<MessageUploadManager>,
        ) = mpsc::channel();

        thread::spawn(move || {
            let mut uploader_manager = UploaderManager::new(rx, config, SharingState::Local);
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

        println!("starting uploader manager");
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
                            let mut single_uploader = SingleUploader::new(rx, single_config);
                            single_uploader.run(stream);
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

enum MessageSingleUploader {

}

struct SingleUploader {
    receiver: Receiver<MessageSingleUploader>,
    config: Config,
    nickname: String,
}

impl SingleUploader {

    pub fn new(receiver: Receiver<MessageSingleUploader>, config: Config) -> SingleUploader {

        let nickname =  String::new();
        return SingleUploader {
            receiver,
            config,
            nickname,
        }
    }

    pub fn run(&mut self, stream: TcpStream) {
        let client_connecting_addr = match stream.peer_addr() {
            Ok(client_connecting_addr) => client_connecting_addr,
            Err(e) => {
                // TODO log e
                self.shutdown(stream);
                return;
            },
        };

        let client_connecting_ip = client_connecting_addr.ip().to_string();

        // Find valid SharedUser
        let mut shared_user: Option<SharedUser> = None;
        for user in self.config.get_shared_users() {
            if user.ip == client_connecting_ip {
                shared_user = Some(user);
                break;
            }
        }
        let shared_user = match shared_user {
            Some(shared_user ) => shared_user,
            None => {
                // TODO log unknown IP
                self.shutdown(stream);
                return;
            },
        };

        if !shared_user.allowed {
            // TODO log
            self.shutdown(stream);
            return;
        }

        let mut t_stream = TransmiticStream::new(stream, shared_user.clone(), self.config.get_local_private_id_bytes());
        t_stream.wait_for_incoming().unwrap();
        panic!("incoming downloader connected");

 



    }

    fn read_receiver(&mut self) {
        match self.receiver.try_recv() {
            Ok(value) => match value {

            }
            Err(e) => match e {
                mpsc::TryRecvError::Empty => return,
                mpsc::TryRecvError::Disconnected => return,  // TODO log. When would this happen?
            }
        }
    }

    fn shutdown(&mut self, stream: TcpStream) {
        match stream.shutdown(Shutdown::Both) {
            Ok(_) => {},
            Err(e) => {
                // TODO log e
            },
        }

        // TODO send id to AppAgg
    }
}