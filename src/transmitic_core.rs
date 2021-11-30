use std::{error::Error, net::SocketAddr, sync::{Arc, Mutex}, thread};

extern crate x25519_dalek;
use ring::{
	signature::{self, KeyPair},
};

use crate::{
	config::{self, Config, ConfigSharedFile}, 
	crypto, 
};

pub struct LocalKeyData {
	pub local_key_pair: signature::Ed25519KeyPair,
	pub local_key_pair_bytes: Vec<u8>,
}

pub struct TransmiticCore {
    config: Config,
    is_first_start: bool,
}

impl TransmiticCore {

    pub fn new() -> Result<TransmiticCore, Box<dyn Error>> {
        let config = Config::new()?;
        let is_first_start = config.is_first_start();

        return Ok(TransmiticCore {
            config: config,
            is_first_start,
        });
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

    pub fn get_sharing_port(&self) -> String {
        return self.config.get_sharing_port();
    }
}
