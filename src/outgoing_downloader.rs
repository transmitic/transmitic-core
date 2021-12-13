use core::time;
use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf}, collections::{VecDeque, HashMap}, ops::Index, net::{SocketAddr, TcpStream, Shutdown}, io::{Write, Read}, convert::TryInto,
};

use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use rand_core::OsRng;
use ring::signature;
use ring::signature::Ed25519KeyPair;
use ring::signature::KeyPair;
use x25519_dalek::{EphemeralSecret, PublicKey};

use crate::{config::{Config, SharedUser}, core_consts::{TRAN_MAGIC_NUMBER, TRAN_API_MAJOR, TRAN_API_MINOR, CONN_ESTABLISH_REQUEST, CONN_ESTABLISH_ACCEPT, CONN_ESTABLISH_REJECT}};

pub struct OutgoingDownloader {
    config: Config,
    path_dir_downloads: PathBuf,
    channel_map: HashMap<String, Sender<MessageSingleDownloader>>,
}

impl OutgoingDownloader {
    pub fn new(config: Config) -> Result<OutgoingDownloader, Box<dyn Error>> {
        create_downloads_dir()?;
        let path_dir_downloads = get_path_dir_downloads()?;
        let channel_map= HashMap::new();

        return Ok(OutgoingDownloader {
            config,
            path_dir_downloads,
            channel_map,
        });
    }

    pub fn start_downloading(&mut self) {
        for user in self.config.get_shared_users() {
            let (sx, rx): (
                Sender<MessageSingleDownloader>,
                Receiver<MessageSingleDownloader>,
            ) = mpsc::channel();

            let mut path_queue_file: PathBuf = self.config.get_path_dir_config();
            path_queue_file.push(format!("{}.txt", user.nickname));
            
            let private_id_bytes = self.config.get_local_private_id_bytes();

            self.channel_map.insert(user.nickname.clone(), sx);

            if path_queue_file.exists() {
                thread::spawn(move || {
                    let mut downloader =
                        SingleDownloader::new(rx, private_id_bytes, user.clone(), path_queue_file);
                    downloader.run();
                    println!(
                        "single downloader thread final exit {}",
                        user.nickname.clone()
                    );  // TODO log
                });
            }
        }
    }
}

fn create_downloads_dir() -> Result<(), std::io::Error> {
    let path = get_path_dir_downloads()?;
    println!("download directory: {:?}", path);
    fs::create_dir_all(path)?;
    return Ok(());
}

fn get_path_dir_downloads() -> Result<PathBuf, std::io::Error> {
    let mut path = env::current_exe()?;
    path.pop();
    path.push("Transmitic Downloads");
    return Ok(path);
}

// Single Downloader

enum MessageSingleDownloader {
    NewConfig {
        private_id_bytes: Vec<u8>,
        shared_user: SharedUser,
    },
    NewDownload(String),
    CancelDownload(String),
    PauseDownloads,
    ResumeDownloads,
}

struct SingleDownloader {
    receiver: Receiver<MessageSingleDownloader>,
    private_key_pair: signature::Ed25519KeyPair,
    shared_user: SharedUser,
    path_queue_file: PathBuf,
    download_queue: VecDeque<String>,
    is_downloading_paused: bool,
}

impl SingleDownloader {
    pub fn new(
        receiver: Receiver<MessageSingleDownloader>,
        private_id_bytes: Vec<u8>,
        shared_user: SharedUser,
        path_queue_file: PathBuf,
    ) -> SingleDownloader {

        // TODO function
        let private_key_pair =
        signature::Ed25519KeyPair::from_pkcs8(private_id_bytes.as_ref()).unwrap();

        let download_queue = VecDeque::new();

        return SingleDownloader {
            receiver,
            private_key_pair,
            shared_user,
            path_queue_file,
            download_queue,
            is_downloading_paused: false,
        };
    }

    pub fn run(&mut self) {

        self.initialize_download_queue();

        while true {
            self.read_receiver();

            if self.is_downloading_paused {
                thread::sleep(time::Duration::from_secs(1));
                continue;
            }

            if self.download_queue.is_empty() {
                thread::sleep(time::Duration::from_secs(1));
                continue;
            }

            // TODO add backoff

            // ------ DOWNLOAD FROM QUEUE

            // CREATE CONNECTION
            let mut remote_address = self.shared_user.ip.clone();
            remote_address.push_str(&format!(":{}", self.shared_user.port.clone()));
            println!("Downloader outgoing {} {}", self.shared_user.nickname, remote_address);

            let remote_socket_address: SocketAddr = match remote_address.parse() {
                Ok(remote_socket_address) => remote_socket_address,
                Err(e) => todo!(), // TODO. This shouldn't happen. log.
            };

            let mut stream = match TcpStream::connect_timeout(&remote_socket_address, time::Duration::from_secs(1)) {
                Ok(stream) => stream,
                Err(e) => {
                    println!("Downloader outgoing Could not connect to '{}': {:?}", remote_socket_address, e); // TODO log
                    thread::sleep(time::Duration::from_secs(1));
                    continue;
                }
            };
            // DO attempt key exchange first, then magic number?
            // Establish Transmitic Connection
            // Send TRAN data
            let mut buffer: [u8; 9] = [0; 4 + 1 + 2 + 2];
            buffer[0..4].copy_from_slice(&TRAN_MAGIC_NUMBER);
            buffer[4] = TRAN_API_MAJOR;
            buffer[5..7].copy_from_slice(&TRAN_API_MINOR.to_be_bytes());
            buffer[7..9].copy_from_slice(&CONN_ESTABLISH_REQUEST.to_be_bytes());  // TODO is the request even needed? obviously it's a request to establish
            
            match stream.write_all(&buffer) {
                Ok(_) => {},
                Err(e) => {
                    println!("{} Conn est failed write {}", self.shared_user.nickname, e.to_string());
                    match stream.shutdown(Shutdown::Both) {
                        Ok(_) => {
                            continue;
                        },
                        Err(e2) => {
                            // TODO
                            println!("{} Conn est failed shutdown. {}", self.shared_user.nickname, e2.to_string());
                            continue;
                        },
                    }
                },
            }

            // Get remote TRAN data
            let mut buffer: [u8; 9] = [0; 4 + 1 + 2 + 2];
            // TODO set read timeout
            match stream.read_exact(&mut buffer) {
                Ok(_) => {},
                Err(e) => {
                    println!("{} Conn est resp failed read. {}", self.shared_user.nickname, e.to_string());
                    match stream.shutdown(Shutdown::Both) {
                        Ok(_) => {
                            continue;
                        },
                        Err(e2) => {
                            // TODO
                            println!("{} Conn est resp failed shutdown. {}", self.shared_user.nickname, e2.to_string());
                            continue;
                        },
                    }
                },
            }

            if buffer[0..4] != TRAN_MAGIC_NUMBER {
                println!("{} TRAN MAGIC NUMBER mismatch. {:?}", self.shared_user.nickname, &buffer[0..4]);
                // TODO log
                match stream.shutdown(Shutdown::Both) {
                    Ok(_) => {
                        continue;
                    },
                    Err(e2) => {
                        // TODO
                        println!("{} TRAN MAGIC NUMBER mismatch failed shutdown. {}", self.shared_user.nickname, e2.to_string());
                        continue;
                    },
                }
            }

            if buffer[4] != TRAN_API_MAJOR {
                // TODO log
                println!("{} TRAN API MAJOR mismatch. {}", self.shared_user.nickname, &buffer[4]);
                match stream.shutdown(Shutdown::Both) {
                    Ok(_) => {
                        continue;
                    },
                    Err(e2) => {
                        // TODO
                        println!("{} TRAN API MAJOR mismatch failed shutdown. {}", self.shared_user.nickname, e2.to_string());
                        continue;
                    },
                }
            }

            let action: u16 = u16::from_be_bytes(buffer[7..9].try_into().unwrap());

            if action == CONN_ESTABLISH_ACCEPT {
                // Do nothing
            }
            else if action == CONN_ESTABLISH_REJECT {
                println!("{} Conn est resp REJECT.", self.shared_user.nickname);
                match stream.shutdown(Shutdown::Both) {
                    Ok(_) => {
                        continue;
                    },
                    Err(e2) => {
                        // TODO
                        println!("{} Conn est resp REJECT failed shutdown. {}", self.shared_user.nickname, e2.to_string());
                        continue;
                    },
                }
            }
            else {
                // Unsupported
                println!("{} Conn est resp UNSUPPORTED '{}'", self.shared_user.nickname, action);
                match stream.shutdown(Shutdown::Both) {
                    Ok(_) => {
                        continue;
                    },
                    Err(e2) => {
                        // TODO
                        println!("{} Conn est resp UNSUPPORTED failed shutdown. {}", self.shared_user.nickname, e2.to_string());
                        continue;
                    },
                }
            }

            panic!("done incoming");

            let path_active_download = match self.download_queue.get(0) {
                Some(path_active_download) => path_active_download.clone(),
                None => todo!(),  // TODO this should never happen. log. Combine with the above isempty check?
            };

            // SECURE STREAM
            // Diffie
            let local_diffie_secret = EphemeralSecret::new(OsRng);
            let local_diffie_public = PublicKey::from(&local_diffie_secret);
            let local_diffie_public_bytes: &[u8; 32] = local_diffie_public.as_bytes();
            let local_diffie_signature_public_bytes = self.private_key_pair.sign(local_diffie_public_bytes);
            let local_diffie_signed_public_bytes = local_diffie_signature_public_bytes.as_ref();

            // SEND DIFFIE

            // get chunk
            // write chunk
            // Perform checks if should keep downloading

            self.read_receiver();

            // Check if this active download has been cancelled
            if !self.download_queue.is_empty() {
                if self.download_queue.get(0) != Some(&path_active_download) {
                    // Stop downloading file
                }
            }

            if self.is_downloading_paused {
                // Stop downloading file
            }
                

        }
    }

    fn read_receiver(&mut self) {
        match self.receiver.try_recv() {
            Ok(value) => match value {
                MessageSingleDownloader::NewConfig {
                    private_id_bytes,
                    shared_user,
                } => todo!(), // TODO
                MessageSingleDownloader::NewDownload(s) => self.download_queue.push_back(s),
                MessageSingleDownloader::CancelDownload(s) => {
                    self.download_queue.retain(|f| f != &s);
                },
                MessageSingleDownloader::PauseDownloads => {
                    self.is_downloading_paused = true;
                },
                MessageSingleDownloader::ResumeDownloads => {
                    self.is_downloading_paused = false;
                }
            },
            Err(e) => match e {
                mpsc::TryRecvError::Empty => return,
                mpsc::TryRecvError::Disconnected => return, // TODO log. When would this happen?
            },
        }
    }

    fn initialize_download_queue(&mut self) {

        let contents = match fs::read_to_string(&self.path_queue_file) {
            Ok(contents) => contents,
            Err(e) => {
                println!("{}", e.to_string()); // TODO log
                return;
            },
        };

        for mut line in contents.lines() {
            line = line.trim();
            if line != "" {
                self.download_queue.push_back(line.to_string());
            }
        }
    }
}
