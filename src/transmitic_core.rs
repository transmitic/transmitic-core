use std::{error::Error, sync::{Arc, RwLock}, collections::{HashMap, VecDeque}, path::PathBuf};

extern crate x25519_dalek;
use ring::{
	signature::{self},
};
use serde::{Serialize, Deserialize};

use crate::{config::{self, Config, ConfigSharedFile, SharedUser}, outgoing_downloader::{OutgoingDownloader}, incoming_uploader::{IncomingUploader, SharingState}, shared_file::{SelectedDownload, RefreshData}, app_aggregator::{AppAggMessage, CompletedMessage, run_app_loop}};

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
    outgoing_downloader: OutgoingDownloader,
    incoming_uploader: IncomingUploader,
    download_state: Arc<RwLock<HashMap<String, SingleDownloadState>>>,
    upload_state: Arc<RwLock<HashMap<String, SingleUploadState>>>,
}





// TODO stream connect timeout
// TODO! review stream, try_clone, set nonblocking
// TODO how to handle non existing files?
// TODO allow empty IP, port, and PublicIDs. "placeholder" users

// TODO add online/offline to state
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SingleUploadState {
    pub nickname: String,
    pub path: String,
    pub percent: u64,
}

pub struct CompletedDownloadState {
    pub path: String,
    pub path_local_disk: String,
}

pub struct SingleDownloadState {
    pub active_download_path: Option<String>,
    pub active_download_percent: u64,
    pub active_download_local_path: Option<String>,
    pub download_queue: VecDeque<String>,
    pub invalid_downloads: Vec<String>,
    pub completed_downloads: Vec<CompletedMessage>,
    pub is_online: bool,
}

impl SingleDownloadState {
    pub fn new() -> SingleDownloadState {

        return SingleDownloadState {
            active_download_path: None,
            active_download_percent: 0,
            active_download_local_path: None,
            download_queue: VecDeque::new(),
            invalid_downloads: Vec::new(),
            completed_downloads: Vec::new(),
            is_online: true,
        }
    }
}

// TODO inconsistent naming: 
//  active download VS in progress
//  owner vs nickname
impl TransmiticCore {

    pub fn new() -> Result<TransmiticCore, Box<dyn Error>> {
        let config = Config::new()?;
        let is_first_start = config.is_first_start();

        let upload_state: HashMap<String, SingleUploadState> = HashMap::new();
        let upload_state_lock = RwLock::new(upload_state);
        let arc_upload_state = Arc::new(upload_state_lock);
        let arc_upload_clone = Arc::clone(&arc_upload_state);

        let download_state: HashMap<String, SingleDownloadState> = HashMap::new();
        let download_state_lock = RwLock::new(download_state);
        let arc_download_state = Arc::new(download_state_lock);
        let arc_clone = Arc::clone(&arc_download_state);

        let app_sender = run_app_loop(arc_clone, arc_upload_clone);

        app_sender.send(AppAggMessage::LogInfo("AppAgg started".to_string()))?;

        let mut outgoing_downloader = OutgoingDownloader::new(config.clone(), app_sender.clone())?;
        outgoing_downloader.start_downloading();

        let incoming_uploader = IncomingUploader::new(config.clone(), app_sender.clone());

        return Ok(TransmiticCore {
            config: config,
            is_first_start,
            sharing_state: SharingState::Off,
            outgoing_downloader,
            incoming_uploader,
            download_state: arc_download_state,
            upload_state: arc_upload_state,
        });
    }

    pub fn add_files(&mut self, files: Vec<String>) -> Result<(), Box<dyn Error>> {
        self.config.add_files(files)?;
        return Ok(());
    }

    pub fn add_new_user(&mut self, new_nickname: String, new_public_id: String, new_ip: String, new_port: String) -> Result<(), Box<dyn Error>> {
        self.config.add_new_user(new_nickname, new_public_id, new_ip, new_port)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        return Ok(());
    }

    pub fn add_user_to_shared(&mut self, nickname: String, file_path: String) -> Result<(), Box<dyn Error>> {
        self.config.add_user_to_shared(nickname, file_path)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        return Ok(());
    }

    pub fn create_new_id(&mut self) -> Result<(), Box<dyn Error>> {
        self.config.create_new_id()?;
        self.incoming_uploader.set_new_config(self.config.clone());
        self.outgoing_downloader.set_new_config(self.config.clone());
        self.outgoing_downloader.set_new_private_id(self.config.get_local_private_id_bytes());
        return Ok(());
    }

    pub fn downloads_cancel_all(&mut self) {
        self.outgoing_downloader.downloads_cancel_all();
    }

    pub fn downloads_cancel_single(&mut self, nickname: String, file_path: String) {
        self.outgoing_downloader.downloads_cancel_single(nickname, file_path);
    }

    pub fn downloads_resume_all(&mut self) {
        self.outgoing_downloader.downloads_resume_all();
    }

    pub fn downloads_pause_all(&mut self) {
        self.outgoing_downloader.downloads_pause_all();
    }

    pub fn downloads_clear_finished(&mut self) {
        let mut lock = self.download_state.write().unwrap();
        for v in lock.values_mut() {
            v.completed_downloads.clear();
        }
    }

    pub fn downloads_clear_invalid(&mut self) {
        let mut lock = self.download_state.write().unwrap();
        for v in lock.values_mut() {
            v.invalid_downloads.clear();
        }
    }

    pub fn download_selected(&mut self, downloads: Vec<SelectedDownload>) -> Result<(), Box<dyn Error>> {
        self.outgoing_downloader.download_selected(downloads)?;
        return Ok(());
    }

    pub fn is_downloading_paused(&self) -> bool {
        return self.outgoing_downloader.is_downloading_paused();
    }

    pub fn set_port(&mut self, port: String) -> Result<(), Box<dyn Error>> {
        self.config.set_port(port)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        return Ok(());
    }

    pub fn get_download_state(&self) -> &Arc<RwLock<HashMap<String, SingleDownloadState>>> {
        return &self.download_state;
    }

    pub fn get_is_first_start(&self) -> bool {
        return self.is_first_start;
    }

    pub fn get_upload_state(&self) -> &Arc<RwLock<HashMap<String, SingleUploadState>>> {
        return &self.upload_state;
    }

    pub fn get_public_id_string(&self) -> String {
        return self.config.get_public_id_string();
    }

    pub fn get_my_sharing_files(&self) -> Vec<ConfigSharedFile> {
        return self.config.get_shared_files();
    }

    pub fn get_my_sharing_state(&self) -> SharingState {
        return self.sharing_state.clone();
    }

    pub fn get_shared_users(&self) -> Vec<SharedUser> {
        return self.config.get_shared_users();
    }

    pub fn get_sharing_port(&self) -> String {
        return self.config.get_sharing_port();
    }

    pub fn refresh_shared_with_me(&mut self) -> Vec<RefreshData> {
        return self.outgoing_downloader.refresh_shared_with_me();
    }

    pub fn remove_file_from_sharing(&mut self, file_path: String) -> Result<(), Box<dyn Error>> {
        self.config.remove_file_from_sharing(file_path)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        return Ok(());
    }

    pub fn remove_user_from_sharing(&mut self, nickname: String, file_path: String) -> Result<(), Box<dyn Error>> {
        self.config.remove_user_from_sharing(nickname, file_path)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        return Ok(());
    }

    pub fn remove_user(&mut self, nickname: String) -> Result<(), Box<dyn Error>> {
        self.config.remove_user(nickname.clone())?;
        self.incoming_uploader.set_new_config(self.config.clone());
        self.outgoing_downloader.set_new_config(self.config.clone());
        self.outgoing_downloader.remove_user(&nickname);
        return Ok(());
    }

    pub fn set_my_sharing_state(&mut self, sharing_state: SharingState) {
        self.incoming_uploader.set_my_sharing_state(sharing_state.clone());
        self.sharing_state = sharing_state;
    }

    pub fn set_user_is_allowed_state(&mut self, nickname: String, is_allowed: bool) -> Result<(), Box<dyn Error>> {
        self.config.set_user_is_allowed_state(nickname, is_allowed)?;
        self.incoming_uploader.set_new_config(self.config.clone());
        return Ok(());
    }

    pub fn update_user(&mut self, nickname: String, new_public_id: String, new_ip: String, new_port: String) -> Result<(), Box<dyn Error>> {
        // TODO support updating the nickanme
        // Need to pull existing name and public id from UI and new nickname and public id
        // If nickname changes, need to write config, stop existing download, wait for existing download to stop, 
        //  change name of download folder, then allow update_user function to return.

        self.config.update_user(nickname.clone(), new_public_id.clone(), new_ip.clone(), new_port.clone())?;
        self.incoming_uploader.set_new_config(self.config.clone());
        self.outgoing_downloader.set_new_config(self.config.clone());
        self.outgoing_downloader.update_user(&nickname)?;
        return Ok(());
    }

    pub fn get_downloads_dir(&self) -> Result<PathBuf, std::io::Error>  {
        return config::get_path_dir_downloads();
    }

}
