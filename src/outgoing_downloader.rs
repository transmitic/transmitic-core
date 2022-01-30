use core::time;
use std::{
    env,
    error::Error,
    fs::{self, metadata, File, OpenOptions},
    path::{Path, PathBuf}, collections::{VecDeque, HashMap}, ops::Index, net::{SocketAddr, TcpStream, Shutdown}, io::{Write, Read, SeekFrom, Seek}, convert::TryInto,
};

use crate::{shared_file::{SharedFile, remove_invalid_files, print_shared_files, SelectedDownload}, utils::get_file_by_path, encrypted_stream::{self, EncryptedStream}, core_consts::{MSG_FILE_SELECTION_CONTINUE, MSG_FILE_FINISHED, MSG_FILE_CHUNK}};

use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use rand_core::OsRng;
use ring::signature;
use ring::signature::Ed25519KeyPair;
use ring::signature::KeyPair;
use x25519_dalek::{EphemeralSecret, PublicKey};

use crate::{config::{Config, SharedUser}, core_consts::{TRAN_MAGIC_NUMBER, TRAN_API_MAJOR, TRAN_API_MINOR, CONN_ESTABLISH_REQUEST, CONN_ESTABLISH_ACCEPT, CONN_ESTABLISH_REJECT, MSG_FILE_LIST, MSG_FILE_LIST_FINAL}, transmitic_stream::TransmiticStream};

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
            self.start_downloading_single_user(user);
        }
    }

    fn start_downloading_single_user(&mut self, user: SharedUser) {
        let (sx, rx): (
            Sender<MessageSingleDownloader>,
            Receiver<MessageSingleDownloader>,
        ) = mpsc::channel();

        // TODO func
        let mut path_queue_file: PathBuf = self.config.get_path_dir_config();
        path_queue_file.push(format!("{}.txt", user.nickname));
        
        let private_id_bytes = self.config.get_local_private_id_bytes();

        if path_queue_file.exists() {
            self.channel_map.insert(user.nickname.clone(), sx);
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

    pub fn download_selected(&mut self, downloads: Vec<SelectedDownload>) -> Result<(), Box<dyn Error>> {

        for download in downloads {
            
            match self.channel_map.get(&download.owner) {
                Some(channel) => {         
                    match channel.send(MessageSingleDownloader::NewDownload(download.path)) {
                        Ok(_) => todo!(),
                        Err(e) => {
                            todo!("log")
                            // TODO the thread must have shut down?
                            //  If so, cleanup already occurred
                            //  But this didn't make it into the queue file
                            //  Queue files should be maintained by main thread?
                        },
                    }
                },
                None => {
                    // TODO func        
                    let mut path_queue_file: PathBuf = self.config.get_path_dir_config();
                    path_queue_file.push(format!("{}.txt", download.owner));

                    match fs::write(path_queue_file.as_os_str(), download.path) {
                        Ok(_) => {
                            for user in self.config.get_shared_users() {
                                if user.nickname == download.owner {
                                    self.start_downloading_single_user(user);
                                    break;
                                }
                            }                            
                        },
                        Err(_) => {
                            // TODO
                            todo!("log")
                        },
                    }
                },
            }


        }


        

        return Ok(());
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

fn get_path_downloads_dir_user(user: &String)  -> Result<PathBuf, std::io::Error> {
    let mut path = get_path_dir_downloads()?;
    path.push(user);
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
    private_id_bytes: Vec<u8>,
    shared_user: SharedUser,
    path_queue_file: PathBuf,
    download_queue: VecDeque<String>,
    is_downloading_paused: bool,
    stop_downloading: bool,
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
            private_id_bytes,
            shared_user,
            path_queue_file,
            download_queue,
            is_downloading_paused: false,
            stop_downloading: false,
        };
    }

    pub fn run(&mut self) {

        self.initialize_download_queue();

        let root_download_dir = get_path_downloads_dir_user(&self.shared_user.nickname).unwrap();
        let mut root_download_dir = root_download_dir.into_os_string().to_str().unwrap().to_string();
        root_download_dir.push_str("/");

        while true {
            self.read_receiver();

            self.stop_downloading = false;

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

            let mut transmitic_stream = TransmiticStream::new(stream, self.shared_user.clone(), self.private_id_bytes.clone());
            let mut encrypted_stream = transmitic_stream.connect().unwrap(); // TODO remove unwrap
            
            // request file list
            let message: u16 = MSG_FILE_LIST;
            let payload: Vec<u8> = Vec::new();
            encrypted_stream.write(message, &payload).unwrap();

            let mut json_bytes: Vec<u8> = Vec::new();
            loop {
                encrypted_stream.read().unwrap();
                let client_message = encrypted_stream.get_message().unwrap();
                json_bytes.extend_from_slice(encrypted_stream.get_payload());

                if client_message == MSG_FILE_LIST_FINAL {
                    break;
                }
            }

            println!("{:?}", json_bytes);
            let files_str = std::str::from_utf8(&json_bytes).unwrap();
            let mut everything_file: SharedFile = serde_json::from_str(&files_str).unwrap();

            remove_invalid_files(&mut everything_file);

            print_shared_files(&everything_file, &"".to_string());
            
            loop {
                let path_active_download = match self.download_queue.get(0) {
                    Some(path_active_download) => path_active_download.clone(),
                    None => {
                        println!("Download queue empty");
                        break;
                    }
                };

                println!("{}", path_active_download);

                // Check if file is valid
                let shared_file = match get_file_by_path(&path_active_download, &everything_file) {
                    Some(file) =>  file,
                    None => {
                        panic!("invlaid file choice");
                        // TODO add invalid download list
                        // TODO remove from download queue
                    }
                };

                self.download_shared_file(&mut encrypted_stream, &shared_file, &root_download_dir, &root_download_dir);

                // TODO what if the download failed? don't pop
                self.download_queue.pop_front();
                self.write_queue();
                
                // TODO
                self.read_receiver();
                if self.stop_downloading {
                    break;
                }
            }

            // TODO shutdown stream

        }
    }

    fn download_shared_file(&mut self, encrypted_stream: &mut EncryptedStream, shared_file: &SharedFile, root_download_dir: &String, download_dir: &String) {
        let current_path_obj = Path::new(&shared_file.path);
        let current_path_name = current_path_obj.file_name().unwrap().to_str().unwrap();

        if shared_file.is_directory {
            let mut new_download_dir = String::from(download_dir);
            new_download_dir.push_str(current_path_name);
            new_download_dir.push_str(&"/".to_string());
            for a_file in &shared_file.files {
                self.download_shared_file(
                    encrypted_stream,
                    a_file,
                    root_download_dir,
                    &new_download_dir,
                );

                // TODO check download cancelled, reset connection
                self.read_receiver();
                if self.stop_downloading {
                    return;
                }
            }

            
        } else {
            // Create directory for file download
            fs::create_dir_all(&download_dir).unwrap();
            let mut destination_path = download_dir.clone();
            destination_path.push_str(current_path_name);
            println!("Saving to: {}", destination_path);

            // Send selection to server
            println!("Sending selection to server");
            let selection_msg: u16;
            let selection_payload: Vec<u8>;
            let file_length: u64;
            if Path::new(&destination_path).exists() {
                file_length = metadata(&destination_path).unwrap().len();
            } else {
                file_length = 0;
            }
            let mut file_continue_payload = file_length.to_be_bytes().to_vec();
            file_continue_payload.extend_from_slice(&shared_file.path.as_bytes());
            selection_msg = MSG_FILE_SELECTION_CONTINUE;
            selection_payload = file_continue_payload;
            encrypted_stream.write(selection_msg, &selection_payload).unwrap();

            // Check first response for error
            encrypted_stream.read().unwrap();
            let remote_message = encrypted_stream.get_message().unwrap();
            println!("{:?}", remote_message);

            // TODO check FILE_INVALID, CANNOT SELECT DIR, MSG_FILE_CHUNK, MSG_FILE_FINISHED
            // And blanket unexpected

            // Valid file, download it
            let mut current_downloaded_bytes: usize;
            let mut f: File;
            // TODO use .create() and remove else?
            if Path::new(&destination_path).exists() {
                f = OpenOptions::new()
                    .write(true)
                    .open(&destination_path)
                    .unwrap();
                f.seek(SeekFrom::End(0)).unwrap();
                current_downloaded_bytes = metadata(&destination_path).unwrap().len() as usize;
            } else {
                f = File::create(&destination_path).unwrap();
                current_downloaded_bytes = 0;
            }

            // TODO update current downloaded bytes

            loop {

                let mut payload_bytes: Vec<u8> = Vec::new();
                payload_bytes.extend_from_slice(encrypted_stream.get_payload());
                current_downloaded_bytes += payload_bytes.len();
                f.write(&payload_bytes).unwrap();

                encrypted_stream.read().unwrap();

                // TODO check reset, cancelled download
                self.read_receiver();
                if self.stop_downloading {
                    return;
                }

                let remote_message = encrypted_stream.get_message().unwrap();
                if remote_message == MSG_FILE_FINISHED {
                    // TODO
                    println!("100%\nDownload finished: {}", destination_path);
                    break;
                }
                if remote_message != MSG_FILE_CHUNK {
                    panic!("Expected MSG_FILE_CHUNK, got {}", remote_message);
                }
            }
        }
    }

    fn read_receiver(&mut self) {
        loop{
            match self.receiver.try_recv() {
                Ok(value) => match value {
                    MessageSingleDownloader::NewConfig {
                        private_id_bytes,
                        shared_user,
                    } => todo!(), // TODO
                    MessageSingleDownloader::NewDownload(s) => {
                        self.download_queue.push_back(s);
                        self.write_queue();
                    },
                    MessageSingleDownloader::CancelDownload(s) => {
                        self.download_queue.retain(|f| f != &s);
                        self.stop_downloading = true;
                        self.write_queue();
                    },
                    MessageSingleDownloader::PauseDownloads => {
                        self.is_downloading_paused = true;
                        self.stop_downloading = true;
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

    fn write_queue(&self) {
		let mut write_string = String::new();

		for f in &self.download_queue {
			write_string.push_str(&format!("{}\n", f));
		}

		let mut f = OpenOptions::new()
			.write(true)
			.create(true)
			.truncate(true)
			.open(&self.path_queue_file)
			.unwrap();
		f.write(write_string.as_bytes()).unwrap();
	}

}
