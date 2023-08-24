use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    fs,
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex, RwLock,
    },
};

extern crate x25519_dalek;
use ring::signature::{self};
use serde::{Deserialize, Serialize};

use crate::{
    app_aggregator::{run_app_loop, AppAggMessage, CompletedMessage},
    config::{Config, ConfigSharedFile, SharedUser},
    incoming_uploader::{IncomingUploader, IncomingUploaderError, SharingState},
    logger::{LogLevel, Logger, DEFAULT_LOG_LEVEL, DEFAULT_LOG_TO_FILE},
    outgoing_downloader::OutgoingDownloader,
    shared_file::{RefreshData, SelectedDownload},
};

// TODO
//  https://doc.rust-lang.org/std/sync/struct.BarrierWaitResult.html
//  https://doc.rust-lang.org/std/sync/struct.Condvar.html

pub struct LocalKeyData {
    pub local_key_pair: signature::Ed25519KeyPair,
    pub local_key_pair_bytes: Vec<u8>,
}

pub struct TransmiticCore {
    config: Config,
    is_first_start: bool,
    sharing_state: SharingState,
    outgoing_downloader: Arc<Mutex<OutgoingDownloader>>,
    incoming_uploader: IncomingUploader,
    incoming_uploader_error: Arc<Mutex<Option<IncomingUploaderError>>>,
    download_state: Arc<RwLock<HashMap<String, SingleDownloadState>>>,
    upload_state: Arc<RwLock<HashMap<String, SingleUploadState>>>,
    app_sender: Sender<AppAggMessage>,
    logger: Arc<Mutex<Logger>>,
    log_path: PathBuf,
}

// TODO allow empty IP, port, and PublicIDs. "placeholder" users

// TODO add online/offline to state
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SingleUploadState {
    pub nickname: String,
    pub path: String,
    pub percent: u64,
    pub is_online: bool,
}

pub struct CompletedDownloadState {
    pub path: String,
    pub path_local_disk: String,
}

pub struct SingleDownloadState {
    pub active_download_path: Option<String>,
    pub active_download_percent: u64,
    pub active_download_local_path: Option<String>,
    pub active_download_size: String,
    pub download_queue: VecDeque<String>,
    pub invalid_downloads: Vec<String>,
    pub completed_downloads: Vec<CompletedMessage>,
    pub is_online: bool,
    pub error: Option<String>,
}

impl Default for SingleDownloadState {
    fn default() -> SingleDownloadState {
        SingleDownloadState {
            active_download_path: None,
            active_download_percent: 0,
            active_download_local_path: None,
            active_download_size: "".to_string(),
            download_queue: VecDeque::new(),
            invalid_downloads: Vec::new(),
            completed_downloads: Vec::new(),
            is_online: true,
            error: None,
        }
    }
}

// TODO inconsistent naming:
//  active download VS in progress
//  owner vs nickname
impl TransmiticCore {
    pub fn new(config: Config) -> Result<TransmiticCore, Box<dyn Error>> {
        let is_first_start = config.is_first_start();

        let is_log_to_file = DEFAULT_LOG_TO_FILE;
        let log_level = DEFAULT_LOG_LEVEL;
        let logger = Logger::new(log_level, is_log_to_file);
        let log_path = logger.get_log_path();
        let logger_arc = Arc::new(Mutex::new(logger));
        let logger_clone = Arc::clone(&logger_arc);

        let upload_state: HashMap<String, SingleUploadState> = HashMap::new();
        let upload_state_lock = RwLock::new(upload_state);
        let arc_upload_state = Arc::new(upload_state_lock);
        let arc_upload_clone = Arc::clone(&arc_upload_state);

        let download_state: HashMap<String, SingleDownloadState> = HashMap::new();
        let download_state_lock = RwLock::new(download_state);
        let arc_download_state = Arc::new(download_state_lock);
        let arc_clone = Arc::clone(&arc_download_state);

        let incoming_uploader_error: Option<IncomingUploaderError> = None;
        let incoming_uploader_error_arc = Arc::new(Mutex::new(incoming_uploader_error));
        let incoming_uploader_error_clone = incoming_uploader_error_arc.clone();

        let (app_sender, app_receiver): (Sender<AppAggMessage>, Receiver<AppAggMessage>) =
            mpsc::channel();

        let mut outgoing_downloader = OutgoingDownloader::new(config.clone(), app_sender.clone())?;
        outgoing_downloader.start_downloading();
        let outgoing_arc = Arc::new(Mutex::new(outgoing_downloader));

        run_app_loop(
            app_receiver,
            arc_clone,
            arc_upload_clone,
            logger_clone,
            incoming_uploader_error_clone,
            outgoing_arc.clone(),
        );

        app_sender.send(AppAggMessage::LogCritical("AppAgg started".to_string()))?;
        app_sender.send(AppAggMessage::LogCritical(format!(
            "{:?}",
            config.get_path_dir_config().as_os_str()
        )))?;

        app_sender.send(AppAggMessage::LogCritical(format!(
            "Is Transmitic installed: {}",
            crate::config::is_transmitic_installed()?
        )))?;

        app_sender.send(AppAggMessage::LogCritical(format!(
            "Downloads dir: {}",
            config.get_path_downloads_dir()?
        )))?;

        let incoming_uploader = IncomingUploader::new(config.clone(), app_sender.clone());

        Ok(TransmiticCore {
            config,
            is_first_start,
            sharing_state: SharingState::Off,
            outgoing_downloader: outgoing_arc,
            incoming_uploader,
            incoming_uploader_error: incoming_uploader_error_arc,
            download_state: arc_download_state,
            upload_state: arc_upload_state,
            app_sender,
            logger: logger_arc,
            log_path,
        })
    }

    pub fn is_config_encrypted(&self) -> bool {
        self.config.is_config_encrypted()
    }

    pub fn decrypt_config(&mut self) -> Result<(), Box<dyn Error>> {
        match self.config.decrypt_config() {
            Ok(o) => Ok(o),
            Err(e) => {
                self.app_sender.send(AppAggMessage::LogError(format!(
                    "Failed to decrypt config '{:?}'",
                    e
                )))?;
                Err(e)?
            }
        }
    }

    pub fn encrypt_config(&mut self, password: String) -> Result<(), Box<dyn Error>> {
        match self.config.encrypt_config(password) {
            Ok(o) => Ok(o),
            Err(e) => {
                self.app_sender.send(AppAggMessage::LogError(format!(
                    "Failed to encrypt config '{:?}'",
                    e
                )))?;
                Err(e)?
            }
        }
    }

    pub fn add_files(&mut self, files: Vec<String>) -> Result<(), Box<dyn Error>> {
        self.config.add_files(files)?;
        Ok(())
    }

    pub fn add_new_user(
        &mut self,
        new_nickname: String,
        new_public_id: String,
        new_ip: String,
        new_port: String,
    ) -> Result<(), Box<dyn Error>> {
        self.app_sender.send(AppAggMessage::LogInfo(format!(
            "Add user '{}'",
            &new_nickname
        )))?;
        self.config
            .add_new_user(new_nickname, new_public_id, new_ip, new_port)?;
        self.incoming_uploader.set_new_config(self.config.clone());

        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.set_new_config(self.config.clone());
        Ok(())
    }

    pub fn add_user_to_shared(
        &mut self,
        nickname: String,
        file_path: String,
    ) -> Result<(), Box<dyn Error>> {
        self.app_sender.send(AppAggMessage::LogInfo(format!(
            "Add user to shared '{}' - {}",
            &nickname, &file_path
        )))?;
        self.config.add_user_to_shared(nickname, file_path)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        Ok(())
    }

    pub fn create_new_id(&mut self) -> Result<(), Box<dyn Error>> {
        self.app_sender
            .send(AppAggMessage::LogWarning("Create new ID".to_string()))?;
        self.config.create_new_id()?;
        self.incoming_uploader.set_new_config(self.config.clone());

        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.set_new_config(self.config.clone());
        Ok(())
    }

    pub fn downloads_cancel_all(&mut self) {
        self.app_sender
            .send(AppAggMessage::LogInfo("Cancel all downloads".to_string()))
            .ok();

        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.downloads_cancel_all();
    }

    pub fn downloads_cancel_single(&mut self, nickname: String, file_path: String) {
        self.app_sender
            .send(AppAggMessage::LogInfo(format!(
                "Cancel single download '{}' - '{}'",
                &nickname, &file_path
            )))
            .ok();

        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.downloads_cancel_single(nickname, file_path);
    }

    pub fn downloads_resume_all(&mut self) {
        self.app_sender
            .send(AppAggMessage::LogInfo("Resume all downloads".to_string()))
            .ok();

        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.downloads_resume_all();
    }

    pub fn downloads_pause_all(&mut self) {
        self.app_sender
            .send(AppAggMessage::LogInfo("Pause all downloads".to_string()))
            .ok();

        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.downloads_pause_all();
    }

    pub fn downloads_clear_finished(&mut self) {
        self.app_sender
            .send(AppAggMessage::LogInfo("Clear downloads".to_string()))
            .ok();
        let mut lock = self.download_state.write().unwrap();
        for v in lock.values_mut() {
            v.completed_downloads.clear();
        }
    }

    pub fn downloads_clear_finished_from_me(&mut self) {
        self.app_sender
            .send(AppAggMessage::LogInfo(
                "Clear downloads finished from me".to_string(),
            ))
            .ok();
        let upload_state = self.get_upload_state();
        let mut u = upload_state.write().unwrap();
        u.clear();
    }

    pub fn downloads_clear_invalid(&mut self) {
        self.app_sender
            .send(AppAggMessage::LogInfo(
                "Cancel invalid downloads".to_string(),
            ))
            .ok();
        let mut lock = self.download_state.write().unwrap();
        for v in lock.values_mut() {
            v.invalid_downloads.clear();
        }
    }

    pub fn download_selected(
        &mut self,
        downloads: Vec<SelectedDownload>,
    ) -> Result<(), Box<dyn Error>> {
        self.app_sender
            .send(AppAggMessage::LogInfo(format!(
                "Download selected '{:?}'",
                &downloads
            )))
            .ok();

        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.download_selected(downloads)?;
        Ok(())
    }

    pub fn is_downloading_paused(&self) -> bool {
        let outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.is_downloading_paused()
    }

    pub fn set_port(&mut self, port: String) -> Result<(), Box<dyn Error>> {
        self.app_sender
            .send(AppAggMessage::LogInfo(format!("Set port '{}'", &port)))
            .ok();
        self.config.set_port(port)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        Ok(())
    }

    pub fn get_download_state(&self) -> &Arc<RwLock<HashMap<String, SingleDownloadState>>> {
        &self.download_state
    }

    pub fn get_is_first_start(&self) -> bool {
        self.is_first_start
    }

    pub fn get_upload_state(&self) -> &Arc<RwLock<HashMap<String, SingleUploadState>>> {
        &self.upload_state
    }

    pub fn get_public_id_string(&self) -> String {
        self.config.get_public_id_string()
    }

    pub fn get_and_reset_my_sharing_error(&mut self) -> Option<IncomingUploaderError> {
        let mut incoming_error_guard = self
            .incoming_uploader_error
            .lock()
            .unwrap_or_else(|err| err.into_inner());

        let error: Option<IncomingUploaderError> = incoming_error_guard.clone();

        *incoming_error_guard = None;
        drop(incoming_error_guard);

        if error.is_some() {
            self.set_my_sharing_state(SharingState::Off);
        }

        error
    }

    pub fn get_my_sharing_files(&self) -> Vec<ConfigSharedFile> {
        self.config.get_shared_files()
    }

    pub fn get_my_sharing_state(&self) -> SharingState {
        self.sharing_state.clone()
    }

    pub fn is_ignore_incoming(&self) -> bool {
        self.config.is_ignore_incoming()
    }

    pub fn is_reverse_connection(&self) -> bool {
        self.config.is_reverse_connection()
    }

    pub fn get_log_path(&self) -> PathBuf {
        self.log_path.clone()
    }

    pub fn get_log_messages(&self) -> Vec<String> {
        let logger_guard = self.logger.lock().unwrap_or_else(|err| err.into_inner());
        logger_guard.get_log_messages()
    }

    pub fn get_log_level(&self) -> LogLevel {
        let logger_guard = self.logger.lock().unwrap_or_else(|err| err.into_inner());
        logger_guard.get_log_level()
    }

    pub fn is_log_to_file(&self) -> bool {
        let logger_guard = self.logger.lock().unwrap_or_else(|err| err.into_inner());
        logger_guard.get_is_file_logging()
    }

    pub fn log_to_file_start(&mut self) {
        self.app_sender
            .send(AppAggMessage::LogToFileState(true))
            .unwrap();
    }

    pub fn log_to_file_stop(&mut self) {
        self.app_sender
            .send(AppAggMessage::LogToFileState(false))
            .unwrap();
    }

    pub fn set_log_level(&mut self, log_level: LogLevel) {
        self.app_sender
            .send(AppAggMessage::SetLogLevel(log_level))
            .unwrap();
    }

    pub fn get_shared_users(&self) -> Vec<SharedUser> {
        self.config.get_shared_users()
    }

    pub fn get_sharing_port(&self) -> String {
        self.config.get_sharing_port()
    }

    pub fn get_shared_with_me_data(&mut self) -> HashMap<String, RefreshData> {
        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.get_shared_with_me_data()
    }

    pub fn get_path_downloads_dir(&self) -> Result<String, Box<dyn Error>> {
        self.config.get_path_downloads_dir()
    }

    pub fn set_path_downloads_dir(&mut self, path: String) -> Result<(), Box<dyn Error>> {
        fs::create_dir_all(&path)?;
        self.config.set_path_downloads_dir(path)?;
        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.set_new_config(self.config.clone());
        Ok(())
    }

    pub fn start_refresh_shared_with_me_all(&mut self) {
        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.start_refresh_shared_with_me_all();
    }

    pub fn start_refresh_shared_with_me_single_user(&mut self, nickname: String) {
        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.start_refresh_shared_with_me_single_user(nickname);
    }

    pub fn remove_file_from_sharing(&mut self, file_path: String) -> Result<(), Box<dyn Error>> {
        self.app_sender
            .send(AppAggMessage::LogInfo(format!(
                "Remove from sharing '{}'",
                &file_path
            )))
            .ok();
        self.config.remove_file_from_sharing(file_path)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        Ok(())
    }

    pub fn remove_user_from_sharing(
        &mut self,
        nickname: String,
        file_path: String,
    ) -> Result<(), Box<dyn Error>> {
        self.app_sender
            .send(AppAggMessage::LogInfo(format!(
                "Remove user from sharing '{}' - '{}'",
                &nickname, &file_path
            )))
            .ok();
        self.config.remove_user_from_sharing(nickname, file_path)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        Ok(())
    }

    pub fn remove_user(&mut self, nickname: String) -> Result<(), Box<dyn Error>> {
        self.app_sender
            .send(AppAggMessage::LogInfo(format!(
                "Remove user '{}'",
                &nickname
            )))
            .ok();
        self.config.remove_user(nickname.clone())?;
        self.incoming_uploader.set_new_config(self.config.clone());

        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.set_new_config(self.config.clone());
        outgoing_guard.remove_user(&nickname);
        Ok(())
    }

    pub fn set_my_sharing_state(&mut self, sharing_state: SharingState) {
        self.app_sender
            .send(AppAggMessage::LogInfo(format!(
                "Set my sharing state '{:?}'",
                &sharing_state
            )))
            .ok();
        self.incoming_uploader
            .set_my_sharing_state(sharing_state.clone());
        self.sharing_state = sharing_state;
        self.app_sender
            .send(AppAggMessage::LogInfo(format!(
                "Sharing state set to '{:?}'",
                self.sharing_state
            )))
            .ok();
    }

    pub fn set_ignore_incoming(&mut self, state: bool) -> Result<(), Box<dyn Error>> {
        self.app_sender
            .send(AppAggMessage::LogInfo(format!(
                "Set ignore incoming '{}'",
                &state
            )))
            .ok();
        self.config.set_ignore_incoming(state)?;
        self.incoming_uploader.set_new_config(self.config.clone());

        // A download could get started by a Reverse Connection, which was allowed in by Ignore Incoming
        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.set_new_config(self.config.clone());
        outgoing_guard.restart_connections();
        Ok(())
    }

    pub fn set_reverse_connection(&mut self, state: bool) -> Result<(), Box<dyn Error>> {
        self.app_sender
            .send(AppAggMessage::LogInfo(format!(
                "Set reverse connection '{}'",
                &state
            )))
            .ok();
        self.config.set_reverse_connection(state)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        Ok(())
    }

    pub fn set_user_is_allowed_state(
        &mut self,
        nickname: String,
        is_allowed: bool,
    ) -> Result<(), Box<dyn Error>> {
        self.app_sender
            .send(AppAggMessage::LogInfo(format!(
                "Set user is allowed state '{}' '{:?}'",
                &nickname, &is_allowed
            )))
            .ok();
        self.config
            .set_user_is_allowed_state(nickname, is_allowed)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        Ok(())
    }

    pub fn update_user(
        &mut self,
        nickname: String,
        new_public_id: String,
        new_ip: String,
        new_port: String,
    ) -> Result<(), Box<dyn Error>> {
        // TODO support updating the nickanme
        // Need to pull existing name and public id from UI and new nickname and public id
        // If nickname changes, need to write config, stop existing download, wait for existing download to stop,
        //  change name of download folder, then allow update_user function to return.
        self.app_sender
            .send(AppAggMessage::LogInfo(format!(
                "Update user '{}' - '{}' - '{}' - '{}'",
                &nickname, &new_public_id, &new_ip, &new_port
            )))
            .ok();
        self.config
            .update_user(nickname.clone(), new_public_id, new_ip, new_port)?;
        self.incoming_uploader.set_new_config(self.config.clone());

        let mut outgoing_guard = self.outgoing_downloader.lock().unwrap();
        outgoing_guard.set_new_config(self.config.clone());
        outgoing_guard.update_user(&nickname)?;
        Ok(())
    }

    pub fn get_downloads_dir(&self) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.config.get_path_downloads_dir()?;
        let path = PathBuf::from(path);
        Ok(path)
    }
}
