use core::time;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::{Read, Write, SeekFrom, Seek};
use std::net::{TcpListener, TcpStream, Shutdown};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use serde::__private::de::TagContentOtherField;

use crate::app_aggregator::AppAggMessage;
use crate::config::{Config, SharedUser, get_everything_file};
use crate::core_consts::{TRAN_MAGIC_NUMBER, TRAN_API_MAJOR, TRAN_API_MINOR, CONN_ESTABLISH_REQUEST, CONN_ESTABLISH_ACCEPT, MSG_FILE_LIST, MSG_FILE_SELECTION_CONTINUE, MAX_DATA_SIZE, MSG_FILE_LIST_FINAL, MSG_FILE_LIST_PIECE, MSG_FILE_INVALID_FILE, MSG_CANNOT_SELECT_DIRECTORY, MSG_FILE_CHUNK, MSG_FILE_FINISHED};
use crate::encrypted_stream::EncryptedStream;
use crate::shared_file::SharedFile;
use crate::transmitic_core::SingleUploadState;
use crate::transmitic_stream::TransmiticStream;
use crate::utils::get_file_by_path;

#[derive(Clone)]
pub enum SharingState {
    Off,
    Local,
    Internet,
}

enum MessageUploadManager {
    SharingStateMsg(SharingState),
}



pub struct IncomingUploader {
    sender: Sender<MessageUploadManager>,
}

impl IncomingUploader {

    pub fn new(config: Config, app_sender: Sender<AppAggMessage>) -> IncomingUploader {
        println!("Starting incoming uploader");
        let (sx, rx): (
            Sender<MessageUploadManager>,
            Receiver<MessageUploadManager>,
        ) = mpsc::channel();

        thread::spawn(move || {
            let mut uploader_manager = UploaderManager::new(rx, config, SharingState::Off, app_sender);
            uploader_manager.run();
        });

        return IncomingUploader {
            sender: sx,
        };
    }

    pub fn set_my_sharing_state(&self, sharing_state: SharingState) {
        self.sender.send(MessageUploadManager::SharingStateMsg(sharing_state)).unwrap();
    }
}


struct UploaderManager {
    stop_incoming: bool,
    receiver: Receiver<MessageUploadManager>,
    config: Config,
    sharing_state: SharingState,
    // TODO once a SingleUplaoder thread is dead, is the reciever dead, and thus can be removed from
    // this vec on next interation? If not, SingleUploader needs to send a message back around.
    // 1. Map usize to Senders. SingleUploader gets its ID. At shutdown send to AppAggreator to pop it
    //  When adding a new one loop through usize until you find available int.
    single_uploaders: Vec<Sender<MessageSingleUploader>>,
    app_sender: Sender<AppAggMessage>,
}

impl UploaderManager {

    pub fn new(receiver: Receiver<MessageUploadManager>, config: Config, sharing_state: SharingState, app_sender: Sender<AppAggMessage>) -> UploaderManager {

        println!("starting uploader manager");
        let single_uploaders = Vec::new();

        return UploaderManager {
            stop_incoming: false,
            receiver: receiver,
            config,
            sharing_state,
            single_uploaders,
            app_sender,
        }
    }

    pub fn run(&mut self) {
        println!("UploaderManager running");

        let mut ip_address;
        loop {
            self.stop_incoming = false;
            self.read_receiver();

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
                self.read_receiver();
                if self.stop_incoming {
                    break;
                }
                match stream {
                    Ok(stream) => {
                        let (sx, rx): (
                            Sender<MessageSingleUploader>,
                            Receiver<MessageSingleUploader>,
                        ) = mpsc::channel();
                        self.single_uploaders.push(sx);
                        let single_config = self.config.clone();
                        let app_sender_clone = self.app_sender.clone();
                        thread::spawn(move || {
                            let mut single_uploader = SingleUploader::new(rx, single_config, app_sender_clone);
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

    fn read_receiver(&mut self) {
        loop {
            match self.receiver.try_recv() {
                Ok(msg) => {
                    match msg {
                        MessageUploadManager::SharingStateMsg(state) => {
                            self.sharing_state = state;
                            self.reset_connections();
                        },
                    }
                },
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => return,
                    mpsc::TryRecvError::Disconnected => return, // TODO log. When would this happen?
                },
            }
        }
    }

    fn reset_connections(&mut self) {
        self.stop_incoming = true;
        for s in &self.single_uploaders {
            s.send(MessageSingleUploader::ShutdownConn);
        }
        self.single_uploaders.clear();
    }
}

enum MessageSingleUploader {
    ShutdownConn,
}

struct SingleUploader {
    receiver: Receiver<MessageSingleUploader>,
    config: Config,
    nickname: String,
    app_sender: Sender<AppAggMessage>,
    should_shutdown: bool,
}

impl SingleUploader {

    pub fn new(receiver: Receiver<MessageSingleUploader>, config: Config, app_sender: Sender<AppAggMessage>) -> SingleUploader {

        let nickname =  String::new();
        return SingleUploader {
            receiver,
            config,
            nickname,
            app_sender,
            should_shutdown: false,
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
        self.app_sender.send(AppAggMessage::StringLog(format!("Incoming Connecting: {}", client_connecting_ip))).unwrap();

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

        self.nickname = shared_user.nickname.clone();

        if !shared_user.allowed {
            // TODO log
            self.shutdown(stream);
            return;
        }

        let mut transmitic_stream = TransmiticStream::new(stream, shared_user.clone(), self.config.get_local_private_id_bytes());
        let mut encrypted_stream = transmitic_stream.wait_for_incoming().unwrap();  // TODO remove unwrap
        println!("enc stream created");

        let everything_file = match get_everything_file(&self.config, &shared_user.nickname) {
            Ok(everything_file) => everything_file,
            Err(e) => {
                println!("{:?}", e);
                return;
            },
        };

        let everything_file_json: String = match serde_json::to_string(&everything_file) {
            Ok(everything_file_json) => everything_file_json,
            Err(e) => {
                println!("{:?}", e);
                return;
            },
        };
        let everything_file_json_bytes = everything_file_json.as_bytes().to_vec();
 
        loop {
            self.read_receiver();
            if self.should_shutdown {
                break;
            }
            match self.run_loop(&mut encrypted_stream, &everything_file, &everything_file_json_bytes) {
                Ok(_) => {},
                Err(e) => {
                    println!("{}", e.to_string());
                    break;
                },
            }
        }
        
        // TODO update Downloading From Me State - Disconnected? Remove from list entirely?
        // TODO shutdown encrypted stream


    }

    fn run_loop(&mut self, encrypted_stream: &mut EncryptedStream, everything_file: &SharedFile, everything_file_json_bytes: &Vec<u8>) -> Result<(), Box<dyn Error>> {

        encrypted_stream.read()?;

        self.read_receiver();
        if self.should_shutdown {
            return Ok(());
        }

        let client_message = encrypted_stream.get_message()?;
        println!("{:?}", client_message);
        println!("{:?}", encrypted_stream.buffer);

        if client_message == MSG_FILE_LIST {
            println!("{:?}", everything_file_json_bytes);
            let mut remaining_bytes = everything_file_json_bytes.len();
            let mut sent_bytes = 0;
            let mut msg;
            let mut payload;
			loop {
				if remaining_bytes <= MAX_DATA_SIZE {
					payload = Vec::from(&everything_file_json_bytes[sent_bytes..remaining_bytes+sent_bytes]);
                    msg = MSG_FILE_LIST_FINAL;
				} else {
					payload = Vec::from(&everything_file_json_bytes[sent_bytes..MAX_DATA_SIZE+sent_bytes]);
                    msg = MSG_FILE_LIST_PIECE;
				}

                match encrypted_stream.write(msg, &payload) {
                    Ok(_) => {},
                    Err(e) => {
                        println!("{:?}", e);
                        return Err(e);
                    },
                }

                self.read_receiver();
                if self.should_shutdown {
                    return Ok(());
                }

                if msg == MSG_FILE_LIST_FINAL {
                    return Ok(());
                }
				
				sent_bytes += MAX_DATA_SIZE;
				remaining_bytes -= MAX_DATA_SIZE;
			}
        }
        else if client_message == MSG_FILE_SELECTION_CONTINUE {
            let mut payload_bytes: Vec<u8> = Vec::new();
            payload_bytes.extend_from_slice(encrypted_stream.get_payload());
            
            let file_seek_point: u64;
            let client_file_choice: &str;
            let mut seek_bytes: [u8; 8] = [0; 8];
            seek_bytes.copy_from_slice(&payload_bytes[0..8]);
            file_seek_point = u64::from_be_bytes(seek_bytes);
            client_file_choice = std::str::from_utf8(&payload_bytes[8..]).unwrap();
    
            println!("    File seek point: {}", file_seek_point);
            println!("    Client chose file {}", client_file_choice);

            // Determine if client's choice is valid
            let client_shared_file = match get_file_by_path(client_file_choice, &everything_file) {
                Some(file) =>  file,
                None => {
                    println!("    ! Invalid file choice");				
                    match encrypted_stream.write(MSG_FILE_INVALID_FILE, &Vec::with_capacity(1)) {
                        Ok(_) => {},
                        Err(e) => {
                            println!("{:?}", e);
                            return Err(e);
                        },
                    }
                    return Ok(());
                }
            };

            // Client cannot select a directory. Client should not allow this to happen.
            if client_shared_file.is_directory {
                println!("    ! Selected directory. Not allowed.");
                match encrypted_stream.write(MSG_CANNOT_SELECT_DIRECTORY, &Vec::with_capacity(1)) {
                    Ok(_) => {},
                    Err(e) => {
                        println!("{:?}", e);
                        return Err(e);
                    },
                }
                return Ok(());
            }

            // Send file to client
            let mut f = OpenOptions::new()
                .read(true)
                .open(client_file_choice)
                .unwrap();
            f.seek(SeekFrom::Start(file_seek_point)).unwrap();

            println!("Start sending file");

            // TODO change to a "Download Starting/Connecting..."
            self.app_sender.send(AppAggMessage::UploadStateChange(SingleUploadState{
                nickname: self.nickname.clone(),
                path: client_file_choice.to_string(),
                percent: 0,
            })).unwrap();

            let mut read_response = 1; // TODO combine with loop?
            let mut read_buffer = [0; MAX_DATA_SIZE];
            let mut current_sent_bytes: usize = file_seek_point as usize;
            let mut download_percent: u64;
            let file_size_f64: f64 = client_shared_file.file_size as f64;
            while read_response != 0 {
                read_response = f.read(&mut read_buffer).unwrap();

                match encrypted_stream.write(MSG_FILE_CHUNK, &read_buffer[0..read_response].to_vec()) {
                    Ok(_) => {},
                    Err(e) => {
                        println!("{:?}", e);
                        return Err(e);
                    },
                }

                current_sent_bytes += read_response;
                download_percent = (((current_sent_bytes as f64) / file_size_f64) * (100 as f64)) as u64;
                println!("{}", download_percent);
                
                self.app_sender.send(AppAggMessage::UploadStateChange(SingleUploadState{
                    nickname: self.nickname.clone(),
                    path: client_file_choice.to_string(),
                    percent: if download_percent < 100 {download_percent} else {99}, // 100 will be sent outside this loop
                })).unwrap();

                self.read_receiver();
                if self.should_shutdown {
                    return Ok(());
                }
            }

            self.app_sender.send(AppAggMessage::UploadStateChange(SingleUploadState{
                nickname: self.nickname.clone(),
                path: client_file_choice.to_string(),
                percent: 100,
            })).unwrap();
            
            // Finished sending file
            println!("Send message finished");
            match encrypted_stream.write(MSG_FILE_FINISHED, &Vec::with_capacity(1)) {
                Ok(_) => {},
                Err(e) => {
                    println!("{:?}", e);
                    return Err(e);
                },
            }
            println!("File transfer complete");

        } else {
            return Err(format!("Invalid client selection {}", client_message.to_string()))?;
        }


        return Ok(());
    }

    fn read_receiver(&mut self) {
        loop {
            match self.receiver.try_recv() {
                Ok(value) => match value {
                    MessageSingleUploader::ShutdownConn => {
                        self.should_shutdown = true;
                    },
                }
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => return,
                    mpsc::TryRecvError::Disconnected => return,  // TODO log. When would this happen?
                }
            }
        }
    }

    // TODO doesn't support EncryptedStream
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