use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

extern crate base64;
use serde::{Deserialize, Serialize};

use crate::crypto;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigSharedFile {
    pub path: String,
    pub shared_with: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SharedUser {
    pub public_id: String,
    pub nickname: String,
    pub ip: String,
    pub port: String,
    pub allowed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ConfigFile {
    pub my_private_id: String,
    pub shared_users: Vec<SharedUser>,
    pub shared_files: Vec<ConfigSharedFile>,
    pub sharing_port: String,
}

pub struct Config {
    first_start: bool,
}

impl Config {
    pub fn new() -> Result<Config, Box<dyn Error>> {
        create_config_dir()?;
        let first_start = init_config()?;

        return Ok(Config { first_start });
    }

    pub fn is_first_start(&self) -> bool {
        return self.first_start;
    }
}

fn create_config_dir() -> Result<bool, std::io::Error> {
    let path = get_path_transmitic_config_dir()?;
    println!("config directory: {:?}", path);
    fs::create_dir_all(path)?;
    return Ok(true);
}

fn get_path_transmitic_config_dir() -> Result<PathBuf, std::io::Error> {
    let mut path = env::current_exe()?;
    path.pop();
    path.push("transmitic_config");
    return Ok(path);
}

fn init_config() -> Result<bool, Box<dyn Error>> {
    let config_path = get_path_config_json()?;
    println!("config path: {:?}", config_path);

    if !config_path.exists() {
        create_new_config()?;
        return Ok(true);
    }

    return Ok(false);
}

fn get_path_config_json() -> Result<PathBuf, std::io::Error> {
    let mut path = get_path_transmitic_config_dir()?;
    path.push("transmitic_config.json");
    return Ok(path);
}

fn create_new_config() -> Result<bool, Box<dyn Error>> {
    let (private_id_bytes, _) = crypto::generate_id_pair().unwrap();

    let private_id_string = base64::encode(private_id_bytes);

    let empty_config: ConfigFile = ConfigFile {
        my_private_id: private_id_string,
        shared_users: Vec::new(),
        shared_files: Vec::new(),
        sharing_port: "7878".to_string(),
    };

    write_config(&empty_config)?;
    return Ok(true);
}

fn write_config(config: &ConfigFile) -> Result<bool, Box<dyn Error>> {
    let config_path = get_path_config_json()?;
    let config_str = serde_json::to_string(&config)?;
    fs::write(config_path, config_str)?;
    return Ok(true);
}
