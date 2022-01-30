use std::{error::Error, net::{SocketAddr, Incoming}, sync::{Arc, Mutex}, thread};

extern crate x25519_dalek;
use ring::{
	signature::{self, KeyPair},
};

use crate::{config::{self, Config, ConfigSharedFile, SharedUser}, crypto, outgoing_downloader::{OutgoingDownloader, self}, incoming_uploader::{IncomingUploader, self, SharingState}, shared_file::SelectedDownload};

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
}

// TODO search for all unwraps
// TODO how to handle non existing files?
// TODO allow empty IP, port, and PublicIDs. "placeholder" users

impl TransmiticCore {

    pub fn new() -> Result<TransmiticCore, Box<dyn Error>> {
        let config = Config::new()?;
        let is_first_start = config.is_first_start();

        let mut outgoing_downloader = OutgoingDownloader::new(config.clone())?;
        outgoing_downloader.start_downloading();

        let mut incoming_uploader = IncomingUploader::new(config.clone());

        return Ok(TransmiticCore {
            config: config,
            is_first_start,
            sharing_state: SharingState::Off,
            outgoing_downloader,
            incoming_uploader,
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

    pub fn download_selected(&mut self, downloads: Vec<SelectedDownload>) -> Result<(), Box<dyn Error>> {
        self.outgoing_downloader.download_selected(downloads)?;
        return Ok(());
    }

    pub fn set_port(&mut self, port: String) -> Result<(), Box<dyn Error>> {
        self.config.set_port(port)?;

        return Ok(());
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

}
