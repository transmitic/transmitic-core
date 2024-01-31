use core::time;
use std::collections::HashMap;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::panic::AssertUnwindSafe;
use std::str::FromStr;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;
use std::{panic, thread};

use crate::app_aggregator::{AppAggMessage, ExitCodes};
use crate::config::{get_everything_file, Config, SharedUser};
use crate::core_consts::{
    DENIED_SLEEP, MAX_DATA_SIZE, MSG_CANNOT_SELECT_DIRECTORY, MSG_FILE_CHUNK, MSG_FILE_FINISHED,
    MSG_FILE_INVALID_FILE, MSG_FILE_LIST, MSG_FILE_LIST_FINAL, MSG_FILE_LIST_PIECE,
    MSG_FILE_SELECTION_CONTINUE, MSG_REVERSE, MSG_REVERSE_NO_LIST,
};
use crate::encrypted_stream::EncryptedStream;
use crate::shared_file::SharedFile;
use crate::transmitic_core::SingleUploadState;
use crate::transmitic_stream::TransmiticStream;
use crate::utils::{get_file_by_path, resolve_address, tcp_connect};

#[cfg(debug_assertions)]
const MAX_REV_SLEEP: i32 = 15;

#[cfg(not(debug_assertions))]
const MAX_REV_SLEEP: i32 = 1800;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SharingState {
    Off,
    Local,
    Internet,
}

// TODO fix this?
#[allow(clippy::large_enum_variant)]
enum MessageUploadManager {
    SharingStateMsg(SharingState),
    NewConfig(Config),
}

#[derive(Clone)]
pub enum IncomingUploaderError {
    PortInUse,
    Generic(String),
}

pub struct IncomingUploader {
    sender: Sender<MessageUploadManager>,
}

impl IncomingUploader {
    pub fn new(config: Config, app_sender: Sender<AppAggMessage>) -> IncomingUploader {
        let (sx, rx): (Sender<MessageUploadManager>, Receiver<MessageUploadManager>) =
            mpsc::channel();
        let sx_clone = sx.clone();

        thread::spawn(move || {
            let upload_sender = sx_clone.clone();
            let app_sender_loop = app_sender.clone();
            let mut uploader_manager =
                UploaderManager::new(rx, config, SharingState::Off, app_sender);
            loop {
                match uploader_manager.run() {
                    Ok(_) => {
                        eprintln!("UploadManager run ended");
                        std::process::exit(ExitCodes::UploadManRunEnd as i32);
                    }
                    Err(e) => {
                        upload_sender
                            .send(MessageUploadManager::SharingStateMsg(SharingState::Off))
                            .ok();

                        let e_string = e.to_string();
                        match e.downcast::<std::io::Error>() {
                            Ok(io_e) if io_e.kind() == std::io::ErrorKind::AddrInUse => {
                                app_sender_loop
                                    .send(AppAggMessage::UploadErrorPortInUse)
                                    .unwrap();
                            }
                            _ => {
                                app_sender_loop
                                    .send(AppAggMessage::UploadErrorGeneric(e_string))
                                    .unwrap();
                            }
                        }
                    }
                }
            }
        });

        IncomingUploader { sender: sx }
    }

    pub fn set_my_sharing_state(&self, sharing_state: SharingState) {
        // Check uploader_manager.run() too
        match self
            .sender
            .send(MessageUploadManager::SharingStateMsg(sharing_state))
        {
            Ok(_) => {}
            Err(e) => {
                eprintln!("UploadManager sender failed {}", e);
                std::process::exit(ExitCodes::UploadManSendFailed as i32);
            }
        }
    }

    pub fn set_new_config(&self, new_config: Config) {
        // Check uploader_manager.run() too
        match self
            .sender
            .send(MessageUploadManager::NewConfig(new_config))
        {
            Ok(_) => {}
            Err(e) => {
                eprintln!("UploadManager sender failed {}", e);
                std::process::exit(ExitCodes::UploadManSendFailed as i32);
            }
        }
    }
}

struct UploaderManager {
    stop_incoming: bool,
    receiver: Receiver<MessageUploadManager>,
    config: Config,
    sharing_state: SharingState,
    single_uploaders: Vec<Sender<MessageSingleUploader>>,
    single_uploaders_rev: HashMap<String, Sender<MessageSingleUploader>>,
    app_sender: Sender<AppAggMessage>,
    // Shared users who have the latest file listing. They do not need to request it.
    // Only used when in Reverse Connection mode
    reverse_file_list_cache: Arc<Mutex<Vec<String>>>,
}

impl UploaderManager {
    pub fn new(
        receiver: Receiver<MessageUploadManager>,
        config: Config,
        sharing_state: SharingState,
        app_sender: Sender<AppAggMessage>,
    ) -> UploaderManager {
        let single_uploaders = Vec::with_capacity(10);
        let single_uploaders_rev: HashMap<String, Sender<MessageSingleUploader>> = HashMap::new();

        // Duped in reset_connection
        let reverse_file_list_vec: Vec<String> =
            Vec::with_capacity(config.get_shared_users().len());
        let reverse_file_list_cache = Arc::new(Mutex::new(reverse_file_list_vec));

        UploaderManager {
            stop_incoming: false,
            receiver,
            config,
            sharing_state,
            single_uploaders,
            single_uploaders_rev,
            app_sender,
            reverse_file_list_cache,
        }
    }

    pub fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let mut ip_addresses: Vec<SocketAddr>;
        loop {
            self.stop_incoming = false;
            self.read_receiver();

            match self.sharing_state {
                SharingState::Off => {
                    thread::sleep(time::Duration::from_secs(1));
                    continue;
                }
                SharingState::Local => {
                    ip_addresses = vec![
                        SocketAddr::from_str(&format!("[::1]:{}", self.config.get_sharing_port()))
                            .unwrap(),
                        SocketAddr::from_str(&format!(
                            "127.0.0.1:{}",
                            self.config.get_sharing_port()
                        ))
                        .unwrap(),
                    ];
                }
                SharingState::Internet => {
                    ip_addresses = vec![
                        SocketAddr::from_str(&format!("[::0]:{}", self.config.get_sharing_port()))
                            .unwrap(),
                        SocketAddr::from_str(&format!(
                            "0.0.0.0:{}",
                            self.config.get_sharing_port()
                        ))
                        .unwrap(),
                    ];
                }
            }

            if self.config.is_reverse_connection() {
                self.app_sender.send(AppAggMessage::LogInfo(
                    "Reverse Connections Starting".to_string(),
                ))?;

                let mut sleep = 1;

                'outer: loop {
                    // Sleep and retry
                    self.app_sender
                        .send(AppAggMessage::LogInfo(format!("Reverse sleep {}", sleep)))?;
                    let mut loop_counter = 1;
                    loop {
                        self.read_receiver();
                        if self.stop_incoming {
                            break 'outer;
                        }

                        if loop_counter >= sleep {
                            break;
                        }
                        loop_counter += 1;
                        thread::sleep(time::Duration::from_secs(1));
                    }
                    // Next sleep
                    sleep *= 3;
                    if sleep > MAX_REV_SLEEP {
                        sleep = MAX_REV_SLEEP;
                    }

                    self.remove_dead_uploaders();

                    for shared_user in self.config.get_shared_users() {
                        let nickname = shared_user.nickname.clone();
                        // Connection in progress
                        if self.single_uploaders_rev.contains_key(&nickname) {
                            continue;
                        }

                        if self.sharing_state == SharingState::Local
                            && (shared_user.ip != "127.0.0.1"
                                && shared_user.ip != "::1"
                                && shared_user.ip != "[::1]")
                        {
                            continue;
                        }

                        self.app_sender.send(AppAggMessage::LogInfo(format!(
                            "Reverse Start {}",
                            &nickname
                        )))?;

                        let (sx, rx): (
                            Sender<MessageSingleUploader>,
                            Receiver<MessageSingleUploader>,
                        ) = mpsc::channel();

                        self.single_uploaders_rev.insert(nickname, sx);

                        let single_config = self.config.clone();
                        let app_sender_clone = self.app_sender.clone();
                        let reverse_file_list_clone = self.reverse_file_list_cache.clone();
                        thread::spawn(move || {
                            let thread_app_sender = app_sender_clone.clone();

                            let remote_socket_address = resolve_address(
                                &shared_user.ip,
                                &shared_user.port,
                                &shared_user,
                                &thread_app_sender,
                            )
                            .unwrap();

                            let stream = match tcp_connect(&remote_socket_address) {
                                Ok(s) => s,
                                Err(_) => {
                                    thread_app_sender
                                        .send(AppAggMessage::LogDebug(format!(
                                            "Reverse offline {}",
                                            shared_user.nickname,
                                        )))
                                        .ok();
                                    return;
                                }
                            };

                            single_thread(
                                stream,
                                Some(shared_user),
                                reverse_file_list_clone,
                                rx,
                                single_config,
                                thread_app_sender,
                            )
                        }); // end thread
                    }
                }
            } else {
                let mut listeners = Vec::new();
                for socket_addr in ip_addresses {
                    match TcpListener::bind(socket_addr) {
                        Ok(listener) => match listener.set_nonblocking(true) {
                            Ok(_) => {
                                self.app_sender
                                    .send(AppAggMessage::LogInfo(format!(
                                        "Waiting for incoming uploads on {}:{}",
                                        listener.local_addr().unwrap().ip(),
                                        listener.local_addr().unwrap().port(),
                                    )))
                                    .ok();
                                listeners.push(listener);
                            }
                            Err(e) => {
                                self.app_sender
                                    .send(AppAggMessage::LogError(format!(
                                        "Failed to set nonblocking on socket {}:{}. {}",
                                        listener.local_addr().unwrap().ip(),
                                        listener.local_addr().unwrap().port(),
                                        e,
                                    )))
                                    .ok();
                            }
                        },
                        Err(e) => {
                            self.app_sender
                                .send(AppAggMessage::LogDebug(format!(
                                    "Failed to bind to socket {}:{}. {}",
                                    socket_addr.ip(),
                                    socket_addr.port(),
                                    e,
                                )))
                                .ok();
                        }
                    }
                }

                if listeners.is_empty() {
                    Err("All TcpListeners failed. No incoming connections can be accepted. See log.")?;
                }

                'all_listeners: loop {
                    for listener in listeners.iter_mut() {
                        self.read_receiver();
                        if self.stop_incoming {
                            break 'all_listeners;
                        }

                        match listener.incoming().next() {
                            Some(stream) => {
                                match stream {
                                    Ok(stream) => {
                                        let (sx, rx): (
                                            Sender<MessageSingleUploader>,
                                            Receiver<MessageSingleUploader>,
                                        ) = mpsc::channel();

                                        self.remove_dead_uploaders();
                                        self.single_uploaders.push(sx);

                                        let single_config = self.config.clone();
                                        let app_sender_clone = self.app_sender.clone();
                                        let reverse_file_list_clone =
                                            self.reverse_file_list_cache.clone();
                                        thread::spawn(move || {
                                            let thread_app_sender = app_sender_clone.clone();
                                            single_thread(
                                                stream,
                                                None,
                                                reverse_file_list_clone,
                                                rx,
                                                single_config,
                                                thread_app_sender,
                                            )
                                        }); // end thread
                                    }
                                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                        thread::sleep(time::Duration::from_millis(375));
                                        continue;
                                    }
                                    Err(e) => {
                                        self.app_sender.send(AppAggMessage::LogDebug(format!(
                                            "Failed initial client connection {}",
                                            e
                                        )))?;
                                        continue;
                                    }
                                }
                            }
                            None => {
                                thread::sleep(time::Duration::from_millis(2000));
                                panic!(
                                    "Listener iterator stopped. {}",
                                    listener.local_addr().unwrap().ip()
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn read_receiver(&mut self) {
        loop {
            match self.receiver.try_recv() {
                Ok(msg) => match msg {
                    MessageUploadManager::SharingStateMsg(state) => {
                        self.sharing_state = state;
                        self.reset_connections();
                    }
                    MessageUploadManager::NewConfig(config) => {
                        self.config = config;
                        self.reset_connections();
                    }
                },
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => return,
                    mpsc::TryRecvError::Disconnected => {
                        eprintln!("UploadManager receiver Disconnected");
                        std::process::exit(ExitCodes::UploadManRecvDisconnected as i32);
                    }
                },
            }
        }
    }

    fn reset_connections(&mut self) {
        self.stop_incoming = true;
        self.send_message_to_all_uploaders(MessageSingleUploader::ShutdownConn);
        self.single_uploaders.clear();
        self.single_uploaders_rev.clear();

        // Duped in fn new
        // We set a _new_ Vec, not just clear it, so that existing threads won't update it out of sync
        let reverse_file_list_vec: Vec<String> =
            Vec::with_capacity(self.config.get_shared_users().len());
        self.reverse_file_list_cache = Arc::new(Mutex::new(reverse_file_list_vec));
    }

    fn remove_dead_uploaders(&mut self) {
        self.send_message_to_all_uploaders(MessageSingleUploader::DoNothing);
    }

    fn send_message_to_all_uploaders(&mut self, message: MessageSingleUploader) {
        self.single_uploaders
            .retain(|uploader| uploader.send(message.clone()).is_ok());

        self.single_uploaders_rev
            .retain(|_, uploader| uploader.send(message.clone()).is_ok());
    }
}

fn single_thread(
    stream: TcpStream,
    reverse_user: Option<SharedUser>,
    reverse_file_list: Arc<Mutex<Vec<String>>>,
    rx: Receiver<MessageSingleUploader>,
    single_config: Config,
    thread_app_sender: Sender<AppAggMessage>,
) {
    let ip = match stream.peer_addr() {
        Ok(addr) => addr.to_string(),
        Err(_) => "Unknown IP".to_string(),
    };
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let mut single_uploader = SingleUploader::new(rx, single_config, thread_app_sender.clone());
        match single_uploader.run(&stream, reverse_user, reverse_file_list) {
            Ok(res) => res,
            Err(e) => {
                thread_app_sender
                    .send(AppAggMessage::LogDebug(format!(
                        "Uploader run error. {} - {}",
                        ip, e
                    )))
                    .unwrap();
                // Mid download. Upload is now disconnected.
                if single_uploader.active_msg == MSG_FILE_SELECTION_CONTINUE {
                    thread_app_sender
                        .send(AppAggMessage::UploadDisconnected(single_uploader.nickname))
                        .unwrap();
                }
                RunLoopResult::Success
            }
        }
    }));

    match result {
        Ok(res) => match res {
            RunLoopResult::Success => {
                stream.shutdown(Shutdown::Both).ok();
            }
            RunLoopResult::ReverseConnection => {}
        },
        Err(e) => {
            thread_app_sender
                .send(AppAggMessage::LogDebug(format!(
                    "Uploader thread error. {} - {:?}",
                    ip, e
                )))
                .unwrap();
            stream.shutdown(Shutdown::Both).ok();
        }
    }
}

#[derive(Clone, Debug)]
enum MessageSingleUploader {
    ShutdownConn,
    DoNothing,
}

enum RunLoopResult {
    Success,
    ReverseConnection,
}

struct SingleUploader {
    receiver: Receiver<MessageSingleUploader>,
    config: Config,
    nickname: String,
    app_sender: Sender<AppAggMessage>,
    should_shutdown: bool,
    active_msg: u16,
}

impl SingleUploader {
    pub fn new(
        receiver: Receiver<MessageSingleUploader>,
        config: Config,
        app_sender: Sender<AppAggMessage>,
    ) -> SingleUploader {
        let nickname = String::new();
        SingleUploader {
            receiver,
            config,
            nickname,
            app_sender,
            should_shutdown: false,
            active_msg: 0,
        }
    }

    // Ok -> An "expected" return. Eg Upload finished and even a user not allowed
    // Error -> An "unexpected" return. Eg failed to parse an IP
    pub fn run(
        &mut self,
        stream: &TcpStream,
        reverse_user: Option<SharedUser>,
        reverse_file_list: Arc<Mutex<Vec<String>>>,
    ) -> Result<RunLoopResult, Box<dyn Error>> {
        let client_connecting_addr = stream.peer_addr()?;

        let client_connecting_ip = client_connecting_addr.ip().to_string();
        self.app_sender.send(AppAggMessage::LogInfo(format!(
            "Incoming Connecting: {}",
            client_connecting_ip
        )))?;

        // Only one valid shared user if Rev Conn
        let mut shared_users = match &reverse_user {
            Some(shared_user) => {
                vec![shared_user.clone()]
            }
            None => self.config.get_shared_users(),
        };

        // Invalid state
        if self.config.is_ignore_incoming() && reverse_user.is_some() {
            eprintln!("Invalid State. Ignore Incoming and Reverse Connection enabled.");
            std::process::exit(ExitCodes::InvalidStateIgnoreAndRev as i32)
        }

        // reverse_user.is_none() is because this isn't a "true incoming". The reverse_user/SharedUser
        //   already has the correct IP to be connecting to, set my the user.
        if !self.config.is_ignore_incoming() && reverse_user.is_none() {
            // Find valid SharedUsers for Transmitic stream to verify
            let mut retained_users: Vec<SharedUser> = Vec::new();
            for shared_user in shared_users {
                match resolve_address(
                    &shared_user.ip,
                    &"0".to_string(),
                    &shared_user,
                    &self.app_sender,
                ) {
                    Ok(addr) => {
                        let mut retain_user: bool = false;
                        for a in addr {
                            if a.ip().to_string() == client_connecting_ip {
                                retain_user = true;
                            } else if a.is_ipv4() && client_connecting_addr.is_ipv6() {
                                let embedded_ip = format!("::ffff:{}", a.ip());
                                if embedded_ip == client_connecting_ip {
                                    retain_user = true;
                                }
                            }

                            if retain_user {
                                retained_users.push(shared_user.clone());
                                break;
                            }
                        }
                    }
                    Err(_) => continue,
                }
            }

            shared_users = retained_users;

            if shared_users.is_empty() {
                self.app_sender.send(AppAggMessage::LogWarning(format!(
                    "Unknown IP tried to connect. Denied.: '{}'",
                    client_connecting_ip
                )))?;
                thread::sleep(time::Duration::from_secs(DENIED_SLEEP));
                return Ok(RunLoopResult::Success);
            }

            // IP matched
            let mut connect_str = String::from("User(s) trying to connect: ");
            for shared_user in shared_users.iter() {
                connect_str.push_str(&format!("'{}', ", shared_user.nickname));
            }
            let connect_str = match connect_str.strip_suffix(", ") {
                Some(s) => s.to_string(),
                None => connect_str,
            };

            self.app_sender.send(AppAggMessage::LogInfo(connect_str))?;
        }

        // TODO duped with refresh_single_user and outgoing downloader
        let mut transmitic_stream = TransmiticStream::new(
            stream.try_clone()?,
            client_connecting_ip.clone(),
            shared_users,
            self.config.get_local_private_id_bytes(),
        );

        let mut encrypted_stream: EncryptedStream;
        match reverse_user {
            Some(user) => {
                let reverse_list_guard = reverse_file_list.lock().unwrap();
                let rev_message = match reverse_list_guard.contains(&user.nickname) {
                    true => MSG_REVERSE_NO_LIST,
                    false => MSG_REVERSE,
                };
                drop(reverse_list_guard);
                encrypted_stream = transmitic_stream.connect()?;
                encrypted_stream.write(rev_message, &Vec::new())?;
            }
            None => {
                encrypted_stream = transmitic_stream.wait_for_incoming()?;
            }
        }
        let shared_user = encrypted_stream.shared_user.clone();
        self.nickname = shared_user.nickname.clone();

        // Not allowed check above too
        if !shared_user.allowed {
            self.process_not_allowed()?;
            return Ok(RunLoopResult::Success);
        }

        self.app_sender.send(AppAggMessage::LogInfo(format!(
            "User successfully connected: '{}' - '{}'",
            self.nickname, client_connecting_ip,
        )))?;

        let everything_file =
            get_everything_file(&self.app_sender, &self.config, &shared_user.nickname)?;
        let everything_file_json: String = serde_json::to_string(&everything_file)?;
        let everything_file_json_bytes = everything_file_json.as_bytes().to_vec();

        loop {
            self.read_receiver()?;
            if self.should_shutdown {
                break;
            }
            match self.run_loop(
                &mut encrypted_stream,
                &everything_file,
                &everything_file_json_bytes,
                &reverse_file_list,
            ) {
                Ok(res) => match res {
                    RunLoopResult::Success => {}
                    RunLoopResult::ReverseConnection => {
                        self.app_sender
                            .send(AppAggMessage::ReverseConnection(encrypted_stream))
                            .unwrap();
                        return Ok(RunLoopResult::ReverseConnection);
                    }
                },
                Err(e) => {
                    return Err(e);
                }
            }
        }

        Ok(RunLoopResult::Success)
    }

    fn process_not_allowed(&self) -> Result<(), Box<dyn Error>> {
        self.app_sender.send(AppAggMessage::LogWarning(format!(
            "User tried to connect but is currently set to Block: '{}'",
            self.nickname
        )))?;
        thread::sleep(time::Duration::from_secs(DENIED_SLEEP));
        Ok(())
    }

    fn run_loop(
        &mut self,
        encrypted_stream: &mut EncryptedStream,
        everything_file: &SharedFile,
        everything_file_json_bytes: &[u8],
        reverse_file_list: &Arc<Mutex<Vec<String>>>,
    ) -> Result<RunLoopResult, Box<dyn Error>> {
        let mut progress_current_time = Instant::now();

        encrypted_stream.read()?;

        self.read_receiver()?;
        if self.should_shutdown {
            return Ok(RunLoopResult::Success);
        }

        let client_message = encrypted_stream.get_message()?;
        self.active_msg = client_message;

        if client_message == MSG_FILE_LIST {
            self.app_sender.send(AppAggMessage::LogDebug(format!(
                "'{}' requested file list",
                self.nickname
            )))?;
            let mut remaining_bytes = everything_file_json_bytes.len();
            let mut sent_bytes = 0;
            let mut msg;
            let mut payload;
            loop {
                if remaining_bytes <= MAX_DATA_SIZE {
                    payload = Vec::from(
                        &everything_file_json_bytes[sent_bytes..remaining_bytes + sent_bytes],
                    );
                    msg = MSG_FILE_LIST_FINAL;
                } else {
                    payload = Vec::from(
                        &everything_file_json_bytes[sent_bytes..MAX_DATA_SIZE + sent_bytes],
                    );
                    msg = MSG_FILE_LIST_PIECE;
                }

                encrypted_stream.write(msg, &payload)?;

                self.read_receiver()?;
                if self.should_shutdown {
                    return Ok(RunLoopResult::Success);
                }

                if msg == MSG_FILE_LIST_FINAL {
                    // -- Reverse list data only used when in Reverse Connection
                    let mut reverse_list_guard = reverse_file_list.lock().unwrap();
                    if !reverse_list_guard.contains(&self.nickname) {
                        reverse_list_guard.push(self.nickname.clone());
                    }
                    drop(reverse_list_guard);
                    // --
                    return Ok(RunLoopResult::Success);
                }

                sent_bytes += MAX_DATA_SIZE;
                remaining_bytes -= MAX_DATA_SIZE;
            }
        } else if client_message == MSG_FILE_SELECTION_CONTINUE {
            let mut payload_bytes: Vec<u8> = Vec::new();
            payload_bytes.extend_from_slice(encrypted_stream.get_payload()?);

            let mut seek_bytes: [u8; 8] = [0; 8];
            seek_bytes.copy_from_slice(&payload_bytes[0..8]);
            let file_seek_point: u64 = u64::from_be_bytes(seek_bytes);
            let client_file_choice: &str = std::str::from_utf8(&payload_bytes[8..])?;

            self.app_sender.send(AppAggMessage::LogDebug(format!(
                "'{}' chose file '{}' at seek point '{}'",
                self.nickname, client_file_choice, file_seek_point
            )))?;

            // Determine if client's choice is valid
            let client_shared_file = match get_file_by_path(client_file_choice, everything_file) {
                Some(file) => file,
                None => {
                    self.app_sender.send(AppAggMessage::LogWarning(format!(
                        "'{}' chose invalid file '{}'",
                        self.nickname, client_file_choice
                    )))?;
                    encrypted_stream.write(MSG_FILE_INVALID_FILE, &Vec::with_capacity(1))?;
                    return Ok(RunLoopResult::Success);
                }
            };

            // Client cannot select a directory. Client should not allow this to happen.
            if client_shared_file.is_directory {
                self.app_sender.send(AppAggMessage::LogWarning(format!(
                    "'{}' chose directory. Not allowed. '{}'",
                    self.nickname, client_file_choice
                )))?;
                encrypted_stream.write(MSG_CANNOT_SELECT_DIRECTORY, &Vec::with_capacity(1))?;
                return Ok(RunLoopResult::Success);
            }

            // Send file to client
            let mut f = OpenOptions::new().read(true).open(client_file_choice)?;
            f.seek(SeekFrom::Start(file_seek_point))?;

            self.app_sender.send(AppAggMessage::LogInfo(format!(
                "Sending '{}' file '{}'",
                self.nickname, client_file_choice
            )))?;

            // TODO change to a "Download Starting/Connecting..."
            self.app_sender
                .send(AppAggMessage::UploadStateChange(SingleUploadState {
                    nickname: self.nickname.clone(),
                    path: client_file_choice.to_string(),
                    percent: 0,
                    is_online: true,
                }))?;

            let mut read_buffer = vec![0; MAX_DATA_SIZE];
            let mut current_sent_bytes: usize = file_seek_point as usize;
            let file_size_f64: f64 = client_shared_file.file_size as f64;
            loop {
                let read_response = f.read(&mut read_buffer)?;
                current_sent_bytes += read_response;

                // Note: Ideally read_response==0 will only happen on empty files. And client should create it itself.
                // If not, that would only happen if expectations of the shared_file and actual file
                // changed, which the client will reject anyway
                let write_message = if current_sent_bytes >= client_shared_file.file_size as usize
                    || read_response == 0
                {
                    MSG_FILE_FINISHED
                } else {
                    MSG_FILE_CHUNK
                };

                encrypted_stream.write(write_message, &read_buffer[0..read_response])?;

                if write_message == MSG_FILE_FINISHED {
                    break;
                }

                // Throttle updates
                if progress_current_time.elapsed().as_secs() > 1 {
                    progress_current_time = Instant::now();

                    let download_percent =
                        (((current_sent_bytes as f64) / file_size_f64) * 100_f64) as u64;

                    self.app_sender
                        .send(AppAggMessage::UploadStateChange(SingleUploadState {
                            nickname: self.nickname.clone(),
                            path: client_file_choice.to_string(),
                            percent: if download_percent < 100 {
                                download_percent
                            } else {
                                99
                            }, // 100 will be sent outside this loop
                            is_online: true,
                        }))?;
                }

                self.read_receiver()?;
                if self.should_shutdown {
                    return Ok(RunLoopResult::Success);
                }
            }

            // Finished sending file
            self.app_sender
                .send(AppAggMessage::UploadStateChange(SingleUploadState {
                    nickname: self.nickname.clone(),
                    path: client_file_choice.to_string(),
                    percent: 100,
                    is_online: true,
                }))?;

            self.app_sender.send(AppAggMessage::LogInfo(format!(
                "File transfer to '{}' completed '{}'",
                self.nickname, client_file_choice
            )))?;
        } else if client_message == MSG_REVERSE || client_message == MSG_REVERSE_NO_LIST {
            encrypted_stream.reverse_message = client_message;
            return Ok(RunLoopResult::ReverseConnection);
        } else {
            return Err(format!(
                "'{}' Invalid client selection '{}'",
                self.nickname, client_message
            )
            .into());
        }

        Ok(RunLoopResult::Success)
    }

    fn read_receiver(&mut self) -> Result<(), Box<dyn Error>> {
        loop {
            match self.receiver.try_recv() {
                Ok(value) => match value {
                    MessageSingleUploader::ShutdownConn => {
                        self.should_shutdown = true;
                    }
                    MessageSingleUploader::DoNothing => {
                        // Don't do anything. Useful to reset the dead uploaders because the Sender fails
                    }
                },
                Err(e) => match e {
                    mpsc::TryRecvError::Empty => return Ok(()),
                    mpsc::TryRecvError::Disconnected => {
                        self.should_shutdown = true;
                        self.app_sender.send(AppAggMessage::LogInfo(format!(
                            "Receiver disconnected '{}' '{}'",
                            self.nickname, e
                        )))?;
                        return Ok(());
                    }
                },
            }
        }
    }
}
