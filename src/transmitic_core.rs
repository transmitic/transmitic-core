use std::{error::Error, net::SocketAddr, sync::{Arc, Mutex}, thread};

extern crate x25519_dalek;
use ring::{
	signature::{self, KeyPair},
};

use crate::{
	config::{self, Config, ConfigSharedFile}, 
	crypto, 
};

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

}
