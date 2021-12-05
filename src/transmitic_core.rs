use std::{error::Error, net::SocketAddr, sync::{Arc, Mutex}, thread};

extern crate x25519_dalek;
use ring::{
	signature::{self, KeyPair},
};

use crate::{config::{self, Config, ConfigSharedFile, SharedUser}, crypto};

pub struct LocalKeyData {
	pub local_key_pair: signature::Ed25519KeyPair,
	pub local_key_pair_bytes: Vec<u8>,
}

pub struct TransmiticCore {
    config: Config,
    is_first_start: bool,
    sharing_state: String,
}

impl TransmiticCore {

    pub fn new() -> Result<TransmiticCore, Box<dyn Error>> {
        let config = Config::new()?;
        let is_first_start = config.is_first_start();

        return Ok(TransmiticCore {
            config: config,
            is_first_start,
            sharing_state: "Off".to_string(),
        });
    }

    pub fn add_new_user(&mut self, new_nickname: String, new_public_id: String, new_ip: String, new_port: String) -> Result<(), Box<dyn Error>> {
        self.config.add_new_user(new_nickname, new_public_id, new_ip, new_port)?;
        return Ok(());
    }

    pub fn create_new_id(&mut self) -> Result<(), Box<dyn Error>> {
        self.config.create_new_id()?;
        return Ok(());
    }

    pub fn set_port(&mut self, port: String) -> Result<(), Box<dyn Error>> {
        self.config.set_port(port)?;

        return Ok(());
    }

    pub fn get_public_id_string(&self) -> String {
        return self.config.get_public_id_string();
    }

    pub fn get_my_sharing_state(&self) -> String {
        return self.sharing_state.clone();
    }

    pub fn get_shared_users(&self) -> Vec<SharedUser> {
        return self.config.get_shared_users();
    }

    pub fn get_sharing_port(&self) -> String {
        return self.config.get_sharing_port();
    }

    pub fn remove_user(&mut self, nickname: String) -> Result<(), Box<dyn Error>> {
        self.config.remove_user(nickname)?;
        return Ok(());
    }

    pub fn set_my_sharing_state(&mut self, sharing_state: String) -> Result<(), Box<dyn Error>> {
        if sharing_state == "Off" {
            
        }
        else if sharing_state == "Local Network" {

        }
        else if sharing_state == "Internet" {

        }
        else {
            return Err(format!("Invalid sharing state {}", sharing_state))?;
        }
        self.sharing_state = sharing_state;
        return Ok(());
    }

    pub fn set_user_is_allowed_state(&mut self, nickname: String, is_allowed: bool) -> Result<(), Box<dyn Error>> {
        self.config.set_user_is_allowed_state(nickname, is_allowed)?;
        return Ok(());
    }
}
