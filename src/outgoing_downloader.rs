use crate::{
    app_aggregator::{
        AppAggMessage, CompletedMessage, InProgressMessage, InvalidFileMessage, OfflineMessage,
    },
    config,
    core_consts::{MSG_FILE_CHUNK, MSG_FILE_FINISHED, MSG_FILE_SELECTION_CONTINUE},
    encrypted_stream::EncryptedStream,
    shared_file::{remove_invalid_files, RefreshData, SelectedDownload, SharedFile},
    utils::get_file_by_path,
};
use core::time;
use std::fmt::Write as _;
use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    fs::{self, metadata, File, OpenOptions},
    io::{ErrorKind, Seek, SeekFrom, Write},
    net::{SocketAddr, TcpStream},
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
};

use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use crate::{
    config::{Config, SharedUser},
    core_consts::{MSG_FILE_LIST, MSG_FILE_LIST_FINAL},
    transmitic_stream::TransmiticStream,
};

const ERR_REC_DISCONNECTED: &str = "Uploader receiver disconnected";

struct InternalRefreshData {
    pub data: Result<SharedFile, String>,
    pub recv: Option<Receiver<RefreshSharedMessages>>,
}

impl InternalRefreshData {
    fn new() -> InternalRefreshData {
        InternalRefreshData {
            data: Err("Refresh".to_string()),
            recv: None,
        }
    }
}

pub enum RefreshSharedMessages {
    FileData(Result<SharedFile, String>),
}

pub struct OutgoingDownloader {
    config: Config,
    channel_map: HashMap<String, Sender<MessageSingleDownloader>>,
    app_sender: Sender<AppAggMessage>,
    is_downloading_paused: bool,
    refresh_data: HashMap<String, InternalRefreshData>,
}

fn set_refresh_data(config: &Config, refresh_data: &mut HashMap<String, InternalRefreshData>) {
    let nicknames: Vec<String> = config
        .get_shared_users()
        .iter()
        .map(|f| f.nickname.clone())
        .collect();

    // Add missing users
    for nickname in nicknames.iter() {
        if !refresh_data.contains_key(nickname) {
            refresh_data.insert(nickname.clone(), InternalRefreshData::new());
        }
    }

    // Remove users who no longer exist
    refresh_data.retain(|k, _| nicknames.contains(k));
}

impl OutgoingDownloader {
    pub fn new(
        config: Config,
        app_sender: Sender<AppAggMessage>,
    ) -> Result<OutgoingDownloader, Box<dyn Error>> {
        create_downloads_dir()?;
        let channel_map = HashMap::with_capacity(10);
        let mut refresh_data = HashMap::with_capacity(10);
        set_refresh_data(&config, &mut refresh_data);

        Ok(OutgoingDownloader {
            config,
            channel_map,
            app_sender,
            is_downloading_paused: false,
            refresh_data,
        })
    }

    pub fn is_downloading_paused(&self) -> bool {
        self.is_downloading_paused
    }

    pub fn start_downloading(&mut self) {
        for user in self.config.get_shared_users() {
            self.start_downloading_single_user(user);
        }
    }

    fn start_downloading_single_user(&mut self, user: SharedUser) {
        // TODO func
        let mut path_queue_file: PathBuf = self.config.get_path_dir_config();
        path_queue_file.push(format!("{}.txt", user.nickname));

        // Don't start download thread if there is nothing to download
        if !path_queue_file.exists() {
            return;
        }

        let private_id_bytes = self.config.get_local_private_id_bytes();
        let app_sender_clone = self.app_sender.clone();
        let is_downloading_paused = self.is_downloading_paused;

        let (sx, rx): (
            Sender<MessageSingleDownloader>,
            Receiver<MessageSingleDownloader>,
        ) = mpsc::channel();
        self.channel_map.insert(user.nickname.clone(), sx);

        thread::spawn(move || {
            let thread_app_sender = app_sender_clone.clone();
            let nickname = user.nickname.clone();
            let mut downloader = SingleDownloader::new(
                rx,
                private_id_bytes,
                user.clone(),
                path_queue_file,
                app_sender_clone,
                is_downloading_paused,
            );

            let mut continue_run = true;
            while continue_run {
                let result = panic::catch_unwind(AssertUnwindSafe(|| {
                    match downloader.run() {
                        Ok(_) => {
                            continue_run = false; // Downloader requests graceful exit
                        }
                        Err(e) => {
                            if e.to_string() == ERR_REC_DISCONNECTED {
                                continue_run = false;
                            }
                            thread_app_sender
                                .send(AppAggMessage::LogDebug(format!(
                                    "Downloader run error. {} - {}",
                                    nickname, e
                                )))
                                .unwrap();
                        }
                    }
                }));

                match result {
                    Ok(_) => {}
                    Err(e) => {
                        // Panic occurred
                        thread_app_sender
                            .send(AppAggMessage::LogDebug(format!(
                                "Downloader run panic. {} - {:?}",
                                nickname, e
                            )))
                            .unwrap();
                    }
                }
            }
            thread_app_sender
                .send(AppAggMessage::LogDebug(format!(
                    "Downloader run loop exit. {}",
                    nickname
                )))
                .unwrap();
        });
    }

    pub fn set_new_config(&mut self, config: Config) {
        self.config = config;
        set_refresh_data(&self.config, &mut self.refresh_data);
    }

    pub fn set_new_private_id(&mut self, private_id_bytes: Vec<u8>) {
        self.send_message_to_all_downloads(MessageSingleDownloader::NewPrivateId(private_id_bytes));
    }

    fn send_message_to_all_downloads(&mut self, message: MessageSingleDownloader) {
        let mut remove_keys: Vec<String> = Vec::new();

        for (key, sender) in self.channel_map.iter_mut() {
            match sender.send(message.clone()) {
                Ok(_) => {}
                Err(_) => remove_keys.push(key.to_string()),
            }
        }

        for key in remove_keys {
            self.channel_map.remove(&key);
        }
    }

    pub fn downloads_cancel_all(&mut self) {
        self.send_message_to_all_downloads(MessageSingleDownloader::CancelAllDownloads);
    }

    #[allow(clippy::single_match)]
    pub fn downloads_cancel_single(&mut self, nickname: String, file_path: String) {
        match self.channel_map.get_mut(&nickname) {
            Some(channel) => {
                match channel.send(MessageSingleDownloader::CancelDownload(file_path)) {
                    Ok(_) => {}
                    Err(_) => {
                        // Downloader died, remove it
                        // TODO but it's still in queue file?
                        // Race
                        self.channel_map.remove(&nickname);
                    }
                }
            }
            None => {
                // TODO could it be left in the queue file?
                // Race
                // Downloader already gone so there is nothing cancel
            }
        }
    }

    #[allow(clippy::single_match)]
    pub fn update_user(&mut self, nickname: &str) -> Result<(), Box<dyn Error>> {
        for shared_user in self.config.get_shared_users() {
            if shared_user.nickname == nickname {
                match self.channel_map.get_mut(nickname) {
                    Some(channel) => {
                        match channel.send(MessageSingleDownloader::NewSharedUser(shared_user)) {
                            Ok(_) => {}
                            Err(_) => {
                                // Downloader died, remove it
                                // TODO I need to restart this
                                self.channel_map.remove(nickname);
                            }
                        }
                    }
                    None => {
                        // TODO should this ever happen? no?
                    }
                }
                return Ok(());
            }
        }

        return Err(format!("Failed to find shared user {}", nickname).into());
    }

    pub fn downloads_pause_all(&mut self) {
        self.is_downloading_paused = true;
        self.send_message_to_all_downloads(MessageSingleDownloader::PauseDownloads);
    }

    pub fn downloads_resume_all(&mut self) {
        self.is_downloading_paused = false;
        self.send_message_to_all_downloads(MessageSingleDownloader::ResumeDownloads);
    }

    pub fn download_selected(
        &mut self,
        downloads: Vec<SelectedDownload>,
    ) -> Result<(), Box<dyn Error>> {
        for download in downloads {
            match self.channel_map.get(&download.owner) {
                Some(channel) => {
                    match channel.send(MessageSingleDownloader::NewDownload(download.path.clone()))
                    {
                        Ok(_) => {}
                        Err(_) => {
                            // Thread shutdown
                            // Append to queue file then start downloader
                            self.add_to_queue_file_and_start_downloader(
                                download.owner,
                                download.path,
                            )?;
                        }
                    }
                }
                None => {
                    self.add_to_queue_file_and_start_downloader(download.owner, download.path)?;
                }
            }
        }

        Ok(())
    }

    pub fn remove_user(&mut self, nickname: &str) {
        // User still exists, else it's already gone, doesn't matter
        if let Some(channel) = self.channel_map.get(nickname) {
            channel.send(MessageSingleDownloader::RemoveUser).ok(); // If it fails thread already died, doesn't matter
        }

        self.channel_map.remove(nickname);
    }

    fn add_to_queue_file_and_start_downloader(
        &mut self,
        download_owner: String,
        download_path: String,
    ) -> Result<(), Box<dyn Error>> {
        // TODO func for path
        let mut path_queue_file: PathBuf = self.config.get_path_dir_config();
        path_queue_file.push(format!("{}.txt", download_owner));

        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(path_queue_file)?;

        // TODO func with write_queue()
        let write_path = format!("{}\n", download_path);
        file.write_all(write_path.as_bytes())?;

        for user in self.config.get_shared_users() {
            if user.nickname == download_owner {
                self.start_downloading_single_user(user);
                break;
            }
        }

        Ok(())
    }

    pub fn start_refresh_shared_with_me_all(&mut self) {
        for shared_user in self.config.get_shared_users() {
            self.start_refresh_shared_with_me_single_user(shared_user.nickname.clone());
        }
    }

    pub fn start_refresh_shared_with_me_single_user(&mut self, nickname: String) {
        let (sx, rx) = mpsc::channel();

        match self.refresh_data.get_mut(&nickname) {
            Some(data) => {
                data.recv = Some(rx);
            }
            None => self
                .app_sender
                .send(AppAggMessage::AppFailedKill(format!(
                    "Start refresh with missing key: '{}'",
                    nickname
                )))
                .unwrap(),
        }
        let config_clone = self.config.clone();

        thread::spawn(move || {
            for shared_user in config_clone.get_shared_users() {
                if shared_user.nickname != nickname {
                    continue;
                }

                let r = refresh_single_user(&config_clone, &shared_user).map_err(|e| e.to_string());
                sx.send(RefreshSharedMessages::FileData(r)).ok();
                break;
            }
        });
    }

    pub fn get_shared_with_me_data(&mut self) -> HashMap<String, RefreshData> {
        for (_, data) in self.refresh_data.iter_mut() {
            match &data.recv {
                Some(recv) => match recv.try_recv() {
                    Ok(msg) => match msg {
                        RefreshSharedMessages::FileData(d) => match d {
                            Ok(shared_file) => {
                                data.data = Ok(shared_file);
                                data.recv = None;
                            }
                            Err(e) => {
                                data.data = Err(e);
                                data.recv = None;
                            }
                        },
                    },
                    Err(e) => match e {
                        mpsc::TryRecvError::Empty => {}
                        mpsc::TryRecvError::Disconnected => {
                            data.data =
                                Err("Transmitic Error: Channel disconnected before refresh data was received."
                                    .to_string());
                            data.recv = None;
                        }
                    },
                },
                None => {}
            }
        }

        let mut refresh_data = HashMap::new();
        for (nickname, data) in self.refresh_data.iter() {
            let in_progress = matches!(data.recv, Some(_));

            refresh_data.insert(
                nickname.clone(),
                RefreshData {
                    owner: nickname.clone(),
                    data: data.data.clone(),
                    in_progress,
                },
            );
        }

        refresh_data
    }
}

fn refresh_single_user(
    config: &Config,
    shared_user: &SharedUser,
) -> Result<SharedFile, Box<dyn Error>> {
    // TODO duped with single downloader
    // CREATE CONNECTION
    let mut remote_address = shared_user.ip.clone();

    write!(remote_address, ":{}", shared_user.port.clone()).unwrap();

    let remote_socket_address: SocketAddr = match remote_address.parse() {
        Ok(remote_socket_address) => remote_socket_address,
        Err(e) => return Err(Box::new(e)),
    };

    let stream = TcpStream::connect_timeout(&remote_socket_address, time::Duration::from_secs(2))?;

    let mut transmitic_stream = TransmiticStream::new(
        stream,
        shared_user.clone(),
        config.get_local_private_id_bytes(),
    );
    let mut encrypted_stream = transmitic_stream.connect()?;

    // request file list
    encrypted_stream.write(MSG_FILE_LIST, &Vec::new())?;

    let mut json_bytes: Vec<u8> = Vec::new();
    let mut client_message: u16;
    loop {
        encrypted_stream.read()?;
        client_message = encrypted_stream.get_message()?;
        json_bytes.extend_from_slice(encrypted_stream.get_payload()?);

        if client_message == MSG_FILE_LIST_FINAL {
            break;
        }
    }

    let files_str = std::str::from_utf8(&json_bytes)?;
    let mut everything_file: SharedFile = serde_json::from_str(files_str)?;

    remove_invalid_files(&mut everything_file);
    //print_shared_files(&everything_file, "");

    Ok(everything_file)
}

fn create_downloads_dir() -> Result<(), std::io::Error> {
    let path = config::get_path_dir_downloads()?;
    fs::create_dir_all(path)?;
    Ok(())
}

fn get_path_downloads_dir_user(user: &str) -> Result<PathBuf, std::io::Error> {
    let mut path = config::get_path_dir_downloads()?;
    path.push(user);
    Ok(path)
}

#[derive(Clone, Debug)]
enum MessageSingleDownloader {
    NewSharedUser(SharedUser),
    NewPrivateId(Vec<u8>),
    NewDownload(String),
    CancelAllDownloads,
    CancelDownload(String),
    PauseDownloads,
    ResumeDownloads,
    RemoveUser,
}

struct SingleDownloader {
    receiver: Receiver<MessageSingleDownloader>,
    private_id_bytes: Vec<u8>,
    shared_user: SharedUser,
    path_queue_file: PathBuf,
    download_queue: VecDeque<String>,
    is_downloading_paused: bool,
    stop_downloading: bool,
    app_sender: Sender<AppAggMessage>,
    active_download_path: Option<String>,
    active_download_size: u64,
    active_downloaded_current_bytes: u64,
    active_download_local_path: Option<String>,
    active_download_total_size: String,
    shutdown: bool,
}

impl SingleDownloader {
    pub fn new(
        receiver: Receiver<MessageSingleDownloader>,
        private_id_bytes: Vec<u8>,
        shared_user: SharedUser,
        path_queue_file: PathBuf,
        app_sender: Sender<AppAggMessage>,
        is_downloading_paused: bool,
    ) -> SingleDownloader {
        let download_queue = VecDeque::new();

        SingleDownloader {
            receiver,
            private_id_bytes,
            shared_user,
            path_queue_file,
            download_queue,
            is_downloading_paused,
            stop_downloading: false,
            app_sender,
            active_download_path: None,
            active_download_size: 0,
            active_downloaded_current_bytes: 0,
            active_download_local_path: None,
            active_download_total_size: "".to_string(),
            shutdown: false,
        }
    }

    // Errors that can't recover: Parsing IP
    // Errors that can recover/restart: Server disconnects mid download
    // How many errors just need to restart the loop, but cause run() to restart?
    // TODO How many errors should just loop again and not exit?
    // TODO downloaders need to end when queue is empty
    //  Race: a new download coming in as existing thread is shutting down
    //    Note: Every download could be sent to every thread but threads only accept matching nickanames?
    //      but could dead threads still be hanging around somewhere causing double downloads?
    // TODO review logic of code that fails that just causes everything to restart anway
    //  eg, if an IP fails to parse it'll all just start again anyway
    //      Auto block? Show error in UI?
    // Error -> An "unexpected" return. Eg bad file path
    #[allow(clippy::while_let_loop)]
    pub fn run(&mut self) -> Result<(), Box<dyn Error>> {
        self.app_sender.send(AppAggMessage::LogInfo(format!(
            "Start downloading from '{}'",
            self.shared_user.nickname
        )))?;
        self.initialize_download_queue()?;
        self.app_update_offline()?;

        let root_download_dir = get_path_downloads_dir_user(&self.shared_user.nickname)?;
        let mut root_download_dir = root_download_dir
            .into_os_string()
            .to_str()
            .unwrap()
            .to_string();
        root_download_dir.push('\\');

        // TODO the inner loop logic in a function that will loop and handle errors to prevent the outer thread loop from restarting the above code?
        loop {
            self.read_receiver()?;

            if self.shutdown {
                self.app_update_in_progress()?;
                break;
            }

            self.stop_downloading = false; // Must come after read_receiver

            if self.is_downloading_paused {
                thread::sleep(time::Duration::from_secs(1));
                continue;
            }

            if self.download_queue.is_empty() {
                thread::sleep(time::Duration::from_millis(500));
                continue;
            }

            // TODO add backoff

            // ------ DOWNLOAD FROM QUEUE

            // TODO duped with refresh_shared_with_me
            // CREATE CONNECTION
            let mut remote_address = self.shared_user.ip.clone();

            write!(remote_address, ":{}", self.shared_user.port.clone()).unwrap();
            self.app_sender.send(AppAggMessage::LogInfo(format!(
                "Attempt outgoing download {} {}",
                self.shared_user.nickname, remote_address
            )))?;

            let remote_socket_address: SocketAddr = remote_address.parse()?;

            let stream = match TcpStream::connect_timeout(
                &remote_socket_address,
                time::Duration::from_secs(2),
            ) {
                Ok(stream) => stream,
                Err(_) => {
                    self.app_update_offline()?;
                    thread::sleep(time::Duration::from_secs(5));
                    continue;
                }
            };

            // TODO duped with refresh_shared_with_me
            let mut transmitic_stream = TransmiticStream::new(
                stream,
                self.shared_user.clone(),
                self.private_id_bytes.clone(),
            );
            let mut encrypted_stream = transmitic_stream.connect()?;

            // request file list
            encrypted_stream.write(MSG_FILE_LIST, &Vec::new())?;

            let mut json_bytes: Vec<u8> = Vec::new();
            let mut client_message: u16;
            loop {
                encrypted_stream.read()?;
                client_message = encrypted_stream.get_message()?;
                json_bytes.extend_from_slice(encrypted_stream.get_payload()?);

                if client_message == MSG_FILE_LIST_FINAL {
                    break;
                }
            }

            let files_str = std::str::from_utf8(&json_bytes)?;
            let mut everything_file: SharedFile = serde_json::from_str(files_str)?;

            remove_invalid_files(&mut everything_file);
            //print_shared_files(&everything_file, "");

            loop {
                let path_active_download = match self.download_queue.get(0) {
                    Some(path_active_download) => path_active_download.clone(),
                    None => {
                        break;
                    }
                };

                // Check if file is valid
                let shared_file = match get_file_by_path(&path_active_download, &everything_file) {
                    Some(file) => file,
                    None => {
                        self.download_queue.pop_front();
                        self.write_queue()?;
                        self.active_download_path = None;
                        self.app_update_invalid_file(&path_active_download)?;
                        continue;
                    }
                };

                self.active_download_path = Some(path_active_download.clone());
                self.active_download_size = shared_file.file_size;
                self.active_downloaded_current_bytes = 0;
                self.active_download_total_size = shared_file.size_string.clone();

                let current_path_name = get_sanitized_disk_file_name(&shared_file)?;
                let mut destination_path = root_download_dir.clone();
                if shared_file.is_directory {
                    destination_path.push_str(&current_path_name);
                }
                self.active_download_local_path = Some(destination_path.clone());

                self.app_update_in_progress()?;
                self.download_shared_file(
                    &mut encrypted_stream,
                    &shared_file,
                    &root_download_dir,
                    &root_download_dir,
                )?;

                // Download was _not_ interrupted, therefore it completed
                if !self.stop_downloading {
                    // TODO duped with above in invalid
                    self.download_queue.pop_front();
                    self.write_queue()?;
                    self.active_download_path = None;
                    self.app_update_completed(
                        &path_active_download,
                        destination_path,
                        self.active_download_total_size.clone(),
                    )?;
                }
                self.app_update_in_progress()?;

                self.read_receiver()?;
                if self.stop_downloading {
                    break;
                }
            }
        }

        Ok(())
    }

    fn download_shared_file(
        &mut self,
        encrypted_stream: &mut EncryptedStream,
        shared_file: &SharedFile,
        root_download_dir: &str,
        download_dir: &str,
    ) -> Result<(), Box<dyn Error>> {
        let current_path_name = get_sanitized_disk_file_name(shared_file)?;

        if shared_file.is_directory {
            let mut new_download_dir = String::from(download_dir);
            new_download_dir.push_str(&current_path_name);
            new_download_dir.push('\\');
            for a_file in &shared_file.files {
                self.download_shared_file(
                    encrypted_stream,
                    a_file,
                    root_download_dir,
                    &new_download_dir,
                )?;

                self.read_receiver()?;
                if self.stop_downloading {
                    return Ok(());
                }
            }
        } else {
            // Create directory for file download
            fs::create_dir_all(&download_dir)?;
            let mut destination_path = download_dir.to_string();
            destination_path.push_str(&current_path_name);
            //println!("Saving to: {}", destination_path);

            // Send selection to server

            let file_length: u64 = if Path::new(&destination_path).exists() {
                metadata(&destination_path)?.len()
            } else {
                0
            };

            let mut file_continue_payload = file_length.to_be_bytes().to_vec();
            file_continue_payload.extend_from_slice(shared_file.path.as_bytes());
            encrypted_stream.write(MSG_FILE_SELECTION_CONTINUE, &file_continue_payload)?;

            // Check first response for error
            encrypted_stream.read()?;
            let remote_message = encrypted_stream.get_message()?;

            if remote_message == MSG_FILE_FINISHED {
                return Ok(());
            } else if remote_message == MSG_FILE_CHUNK {
                // Expected, most likely
            } else {
                // TODO API mismatch, downloader should be disabled
                return Err(format!(
                    "{} Initial file download unexpected msg {}",
                    self.shared_user.nickname, remote_message
                )
                .into());
            }

            // Valid file, download it
            let mut current_downloaded_bytes: usize;
            let mut f: File;
            // TODO use .create() and remove else?
            if Path::new(&destination_path).exists() {
                f = OpenOptions::new().write(true).open(&destination_path)?;
                f.seek(SeekFrom::End(0))?;
                current_downloaded_bytes = metadata(&destination_path)?.len() as usize;
            } else {
                f = File::create(&destination_path)?;
                current_downloaded_bytes = 0;
            }

            self.active_downloaded_current_bytes += current_downloaded_bytes as u64;
            self.app_update_in_progress()?;

            loop {
                let mut payload_bytes: Vec<u8> = Vec::new();
                payload_bytes.extend_from_slice(encrypted_stream.get_payload()?);
                current_downloaded_bytes += payload_bytes.len();
                self.active_downloaded_current_bytes += payload_bytes.len() as u64;

                f.write_all(&payload_bytes)?;

                self.app_update_in_progress()?;

                encrypted_stream.read()?;

                self.read_receiver()?;
                if self.stop_downloading {
                    return Ok(());
                }

                let remote_message = encrypted_stream.get_message()?;
                if remote_message == MSG_FILE_FINISHED {
                    break;
                }
                if remote_message != MSG_FILE_CHUNK {
                    // TODO API mismatch, downloader should be disabled
                    return Err(format!(
                        "{} Mid file download unexpected msg {}",
                        self.shared_user.nickname, remote_message
                    )
                    .into());
                }
            }
        }

        Ok(())
    }

    fn read_receiver(&mut self) -> Result<(), Box<dyn Error>> {
        loop {
            match self.receiver.try_recv() {
                Ok(value) => match value {
                    MessageSingleDownloader::NewPrivateId(private_id_bytes) => {
                        self.stop_downloading = true;
                        self.private_id_bytes = private_id_bytes;

                        self.app_sender.send(AppAggMessage::LogDebug(format!(
                            "Downloader NewPrivateId {}",
                            self.shared_user.nickname.clone()
                        )))?;
                    }
                    MessageSingleDownloader::NewDownload(s) => {
                        self.download_queue.push_back(s.clone());
                        self.write_queue()?;

                        self.app_sender.send(AppAggMessage::LogDebug(format!(
                            "Downloader new download {} - {}",
                            self.shared_user.nickname.clone(),
                            s
                        )))?;
                    }
                    MessageSingleDownloader::CancelDownload(s) => {
                        self.stop_downloading = true;
                        self.download_queue.retain(|f| f != &s);
                        if self.active_download_path == Some(s.clone()) {
                            self.active_download_path = None;
                        }
                        self.write_queue()?;
                        self.app_update_in_progress()?;

                        self.app_sender.send(AppAggMessage::LogDebug(format!(
                            "Downloader cancel download {} - {}",
                            self.shared_user.nickname.clone(),
                            s
                        )))?;
                    }
                    MessageSingleDownloader::PauseDownloads => {
                        self.stop_downloading = true;
                        self.is_downloading_paused = true;

                        self.app_sender.send(AppAggMessage::LogDebug(format!(
                            "Downloader pause {}",
                            self.shared_user.nickname.clone()
                        )))?;
                    }
                    MessageSingleDownloader::ResumeDownloads => {
                        self.is_downloading_paused = false;

                        self.app_sender.send(AppAggMessage::LogDebug(format!(
                            "Downloader resume {}",
                            self.shared_user.nickname.clone()
                        )))?;
                    }
                    MessageSingleDownloader::CancelAllDownloads => {
                        self.stop_downloading = true;
                        self.download_queue.clear();
                        self.active_download_path = None;
                        self.write_queue()?;
                        self.app_update_in_progress()?;

                        self.app_sender.send(AppAggMessage::LogDebug(format!(
                            "Downloader cancel all downloads {}",
                            self.shared_user.nickname.clone()
                        )))?;
                    }
                    MessageSingleDownloader::RemoveUser => {
                        self.shutdown = true;
                        self.stop_downloading = true;
                        self.download_queue.clear();
                        self.active_download_path = None;
                        self.write_queue()?;

                        self.app_sender.send(AppAggMessage::LogDebug(format!(
                            "Downloader remove {}",
                            self.shared_user.nickname.clone()
                        )))?;
                    }
                    MessageSingleDownloader::NewSharedUser(s) => {
                        self.stop_downloading = true;
                        self.shared_user = s;

                        self.app_sender.send(AppAggMessage::LogDebug(format!(
                            "Downloader update user {}",
                            self.shared_user.nickname.clone()
                        )))?;
                    }
                },
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => return Ok(()),
                    mpsc::TryRecvError::Disconnected => return Err(ERR_REC_DISCONNECTED.into()),
                },
            }
        }
    }

    // TODO combine update_offline and in_progress into 1 method, and the downloader struct should track is_offline
    //  then remove app_update_in_progress from read_receiver?
    fn app_update_offline(&self) -> Result<(), Box<dyn Error>> {
        let i = OfflineMessage {
            nickname: self.shared_user.nickname.clone(),
            download_queue: self.download_queue.clone(),
        };
        self.app_sender.send(AppAggMessage::Offline(i))?;

        Ok(())
    }

    fn app_update_in_progress(&self) -> Result<(), Box<dyn Error>> {
        let i = InProgressMessage {
            nickname: self.shared_user.nickname.clone(),
            path: self.active_download_path.clone(),
            percent: ((self.active_downloaded_current_bytes as f64
                / self.active_download_size as f64)
                * 100_f64) as u64,
            download_queue: self.get_queue_without_active(),
            path_local_disk: self.active_download_local_path.clone(),
            size_string: self.active_download_total_size.clone(),
        };
        self.app_sender.send(AppAggMessage::InProgress(i))?;

        Ok(())
    }

    fn app_update_completed(
        &self,
        path: &str,
        path_local_disk: String,
        size: String,
    ) -> Result<(), Box<dyn Error>> {
        let i = CompletedMessage {
            nickname: self.shared_user.nickname.clone(),
            path: path.to_string(),
            download_queue: self.get_queue_without_active(),
            path_local_disk,
            size_string: size,
        };
        self.app_sender.send(AppAggMessage::Completed(i))?;

        Ok(())
    }

    fn app_update_invalid_file(&self, path: &str) -> Result<(), Box<dyn Error>> {
        let i = InvalidFileMessage {
            nickname: self.shared_user.nickname.clone(),
            active_path: self.active_download_path.clone(),
            invalid_path: path.to_string(),
            download_queue: self.get_queue_without_active(),
        };
        self.app_sender.send(AppAggMessage::InvalidFile(i))?;

        Ok(())
    }

    fn get_queue_without_active(&self) -> VecDeque<String> {
        let mut queue = self.download_queue.clone();
        match &self.active_download_path {
            Some(path) => queue.retain(|f| f != path),
            None => {}
        }

        queue
    }

    fn initialize_download_queue(&mut self) -> Result<(), Box<dyn Error>> {
        self.download_queue.clear();
        let contents = fs::read_to_string(&self.path_queue_file)?;

        for mut line in contents.lines() {
            line = line.trim();
            if !line.is_empty() {
                self.download_queue.push_back(line.to_string());
            }
        }

        Ok(())
    }

    fn write_queue(&self) -> Result<(), Box<dyn Error>> {
        let write_string = String::new();

        // Delete queue file so that on program restart, downloader does not start and then do nothing
        if self.download_queue.is_empty() {
            let path_queue = Path::new(&self.path_queue_file);

            match fs::remove_file(path_queue) {
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => {
                    // Failed to delete, so attempt to write empty queue file
                    self.write_queue_file(write_string)?;

                    // TODO create trait that add log_err, log_debug etc to simplify all these calls?
                    self.app_sender.send(AppAggMessage::LogError(format!(
                        "Failed to delete queue file '{}'. {}",
                        path_queue.to_string_lossy(),
                        e
                    )))?;
                }
            }
        } else {
            self.write_queue_file(write_string)?;
        }

        Ok(())
    }

    fn write_queue_file(&self, mut write_string: String) -> Result<(), Box<dyn Error>> {
        for f in &self.download_queue {
            writeln!(write_string, "{}", f).unwrap();
        }

        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path_queue_file)?;
        f.write_all(write_string.as_bytes())?;

        Ok(())
    }
}

fn get_sanitized_disk_file_name(shared_file: &SharedFile) -> Result<String, Box<dyn Error>> {
    let sanitized_path = sanitize_disk_path(&shared_file.path)?;
    let current_path_obj = Path::new(&sanitized_path);
    let current_path_name = current_path_obj
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    Ok(current_path_name)
}

fn sanitize_disk_path(path: &str) -> Result<String, Box<dyn Error>> {
    let mut counter = 0;
    let max_attempts = 10;
    let original_path = path.to_string();
    let mut compare_path = original_path.clone();
    let mut new_path = compare_path.clone();

    while counter < max_attempts {
        counter += 1;
        compare_path = new_path.clone();

        // Replace null bytes
        new_path = new_path.replace('\0', "0");
        if new_path != compare_path {
            continue;
        }

        // Replace the network path, double slash, with a single slash
        new_path = new_path.replace("\\\\", "\\");
        if new_path != compare_path {
            continue;
        }

        // Nothing has changed, path sanitized
        return Ok(new_path);
    }

    return Err(format!(
        "Path could not be sanitized after {} tries. {}",
        max_attempts, original_path
    )
    .into());
}
