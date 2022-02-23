use std::{error::Error, net::{SocketAddr, Incoming}, sync::{Arc, Mutex, RwLock, mpsc::Sender}, thread, collections::{HashMap, VecDeque}, hash::Hash, path::PathBuf};

extern crate x25519_dalek;
use aes_gcm::aead::heapless::spsc::SingleCore;
use ring::{
	signature::{self, KeyPair},
};
use serde::{Serialize, Deserialize};

use crate::{config::{self, Config, ConfigSharedFile, SharedUser}, crypto, outgoing_downloader::{OutgoingDownloader, self}, incoming_uploader::{IncomingUploader, self, SharingState}, shared_file::{SelectedDownload, RefreshData}, app_aggregator::{AppAggregator, AppAggMessage}};

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
    app_sender: Sender<AppAggMessage>,
    download_state: Arc<RwLock<HashMap<String, SingleDownloadState>>>,
    upload_state: Arc<RwLock<HashMap<String, SingleUploadState>>>,
}

struct DownloadStatus {
    pub owner: String,
    pub percent: u32,
    pub path: String,
}

impl DownloadStatus {

    pub fn new() -> DownloadStatus {
        return DownloadStatus {
            owner: "".to_string(),
            percent: 0,
            path: "Hello".to_string(),
        }
    }

    pub fn addit(&mut self) {
        self.owner = "bye".to_string();
    }
}

// Dynamic: In Progress, Queued
// Static: Finished, Invalid

// Downloader Sends
// nickname: String
// inprogress_path: String
// inprogress_percent: usize
// queue: list<String>
// completed: string
// invalid: string

struct TotalDownloadState {
    pub in_progress: Vec<DownloadStatus>,
    pub invalid: Vec<DownloadStatus>,
}

// struct DownloadState {
//     state_map: HashMap<String, SingleDownloadState>,
// }

// impl DownloadState {

//     pub fn new() -> DownloadState {

//         return DownloadState {

//         }
//     }
// }


// TODO search for all unwraps
// TODO how to handle non existing files?
// TODO allow empty IP, port, and PublicIDs. "placeholder" users

// TODO add online/offline to state
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SingleUploadState {
    pub nickname: String,
    pub path: String,
    pub percent: u64,
}

pub struct SingleDownloadState {
    pub active_download_path: Option<String>,
    pub active_download_percent: u64,
    pub download_queue: VecDeque<String>,
    pub invalid_downloads: Vec<String>,
    pub completed_downloads: Vec<String>,
    pub is_online: bool,
}

impl SingleDownloadState {
    pub fn new() -> SingleDownloadState {

        return SingleDownloadState {
            active_download_path: None,
            active_download_percent: 0,
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

        let mut app_agg = AppAggregator::new();
        let mut app_sender = app_agg.start(arc_clone, arc_upload_clone);

        app_sender.send(AppAggMessage::StringLog("AppAgg started".to_string()))?;

        let mut outgoing_downloader = OutgoingDownloader::new(config.clone(), app_sender.clone())?;
        outgoing_downloader.start_downloading();

        let mut incoming_uploader = IncomingUploader::new(config.clone(), app_sender.clone());

        return Ok(TransmiticCore {
            config: config,
            is_first_start,
            sharing_state: SharingState::Off,
            outgoing_downloader,
            incoming_uploader,
            app_sender,
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
        return Ok(());
    }

    pub fn add_user_to_shared(&mut self, nickname: String, file_path: String) -> Result<(), Box<dyn Error>> {
        self.config.add_user_to_shared(nickname, file_path)?;
        return Ok(());
    }

    pub fn create_new_id(&mut self) -> Result<(), Box<dyn Error>> {
        self.config.create_new_id()?;
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

        return Ok(());
    }

    pub fn get_download_state(&self) -> &Arc<RwLock<HashMap<String, SingleDownloadState>>> {
        return &self.download_state;
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
        return Ok(());
    }

    pub fn remove_user_from_sharing(&mut self, nickname: String, file_path: String) -> Result<(), Box<dyn Error>> {
        self.config.remove_user_from_sharing(nickname, file_path)?;
        return Ok(());
    }

    pub fn remove_user(&mut self, nickname: String) -> Result<(), Box<dyn Error>> {
        self.config.remove_user(nickname)?;
        return Ok(());
    }

    pub fn set_my_sharing_state(&mut self, sharing_state: SharingState) {
        self.incoming_uploader.set_my_sharing_state(sharing_state.clone());
        self.sharing_state = sharing_state;
    }

    pub fn set_user_is_allowed_state(&mut self, nickname: String, is_allowed: bool) -> Result<(), Box<dyn Error>> {
        self.config.set_user_is_allowed_state(nickname, is_allowed)?;
        return Ok(());
    }

    pub fn start_my_downloading(&mut self) -> Result<(), Box<dyn Error>> {

        return Ok(());
    }

    pub fn update_user(&mut self, nickname: String, new_public_id: String, new_ip: String, new_port: String) -> Result<(), Box<dyn Error>> {
        // TODO support updating the nickanme
        // Need to pull existing name and public id from UI and new nickname and public id
        // If nickname changes, need to write config, stop existing download, wait for existing download to stop, 
        //  change name of download folder, then allow update_user function to return.

        self.config.update_user(nickname, new_public_id, new_ip, new_port)?;

        return Ok(());
    }

    pub fn get_downloads_dir(&self) -> Result<PathBuf, std::io::Error>  {
        return config::get_path_dir_downloads();
    }

}
