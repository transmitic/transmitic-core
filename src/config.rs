use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;

extern crate base64;
use aes_gcm::aead::generic_array::typenum::private::IsNotEqualPrivate;
use ring::signature;
use ring::signature::KeyPair;
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
    my_private_id: String,
    shared_users: Vec<SharedUser>,
    shared_files: Vec<ConfigSharedFile>,
    sharing_port: String,
}

pub struct Config {
    first_start: bool,
    config_file: ConfigFile,
	local_key_pair: signature::Ed25519KeyPair,
	local_private_key_bytes: Vec<u8>,
}

impl Config {
    pub fn new() -> Result<Config, Box<dyn Error>> {
        create_config_dir()?;
        let first_start = init_config()?;
        let config_file = read_config()?;

        // Load local private key pair
        // TODO repeated in create_new_id and create new config?
        let local_private_key_bytes = crypto::get_bytes_from_base64_str(&config_file.my_private_id)?;
        let local_key_pair =
            signature::Ed25519KeyPair::from_pkcs8(local_private_key_bytes.as_ref()).unwrap();

        return Ok(Config { first_start, config_file, local_key_pair, local_private_key_bytes });
    }

    pub fn add_new_user(&mut self, new_nickname: String, new_public_id: String, new_ip: String, new_port: String) -> Result<(), Box<dyn Error>> {
        let mut new_config_file = self.config_file.clone();
        let shared_user = SharedUser {
            public_id: new_public_id,
            nickname: new_nickname,
            ip: new_ip,
            port: new_port,
            allowed: true,
        };
        new_config_file.shared_users.push(shared_user);
        self.write_and_set_config(new_config_file)?;

        return Ok(());
    }

    pub fn create_new_id(&mut self) -> Result<(), Box<dyn Error>> {
        let (private_id_bytes, _) = crypto::generate_id_pair().unwrap();
        let private_id_string = base64::encode(private_id_bytes);
        
        let mut new_config_file = self.config_file.clone();
        new_config_file.my_private_id = private_id_string;
        self.write_and_set_config(new_config_file)?;

        // TODO move this to write_and_set_config?
        let local_private_key_bytes = crypto::get_bytes_from_base64_str(&self.config_file.my_private_id)?;
        let local_key_pair =
            signature::Ed25519KeyPair::from_pkcs8(local_private_key_bytes.as_ref()).unwrap();
        self.local_key_pair = local_key_pair;
        self.local_private_key_bytes = local_private_key_bytes;

        return Ok(());
    }

    pub fn get_public_id_string(&self) -> String {
        let public_id = self.local_key_pair.public_key().as_ref();
		let public_id_string = crypto::get_base64_str_from_bytes(public_id.to_vec());
        return public_id_string;
    }

    pub fn get_sharing_port(&self) -> String {
        return self.config_file.sharing_port.clone();
    }

    pub fn is_first_start(&self) -> bool {
        return self.first_start;
    }

    pub fn set_port(&mut self, port: String) -> Result<(), Box<dyn Error>> {
        let mut new_config_file = self.config_file.clone();
        new_config_file.sharing_port = port;
        self.write_and_set_config(new_config_file)?;

        return Ok(());
    }

    fn write_and_set_config(&mut self, config_file: ConfigFile) -> Result<(), Box<dyn Error>> {
        write_config(&config_file)?;
        self.config_file = config_file;

        return Ok(());
    }
}

fn create_config_dir() -> Result<(), std::io::Error> {
    let path = get_path_transmitic_config_dir()?;
    println!("config directory: {:?}", path);
    fs::create_dir_all(path)?;
    return Ok(());
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

fn create_new_config() -> Result<(), Box<dyn Error>> {
    let (private_id_bytes, _) = crypto::generate_id_pair().unwrap();

    let private_id_string = base64::encode(private_id_bytes);

    let empty_config: ConfigFile = ConfigFile {
        my_private_id: private_id_string,
        shared_users: Vec::new(),
        shared_files: Vec::new(),
        sharing_port: "7878".to_string(),
    };

    write_config(&empty_config)?;
    return Ok(());
}

fn write_config(config_file: &ConfigFile) -> Result<(), Box<dyn Error>> {
    // TODO Sanitize the JSON.
    // trim all strings
    // should verify_config call sanitize and return the new config?
    //  would this always support reading and writing without issue?

    verify_config(config_file)?;
    let config_path = get_path_config_json()?;
    let config_str = serde_json::to_string(&config_file)?;
    fs::write(config_path, config_str)?;
    return Ok(());
}

fn verify_config(config_file: &ConfigFile) -> Result<(), Box<dyn Error>> {
    verify_config_port(&config_file.sharing_port)?;
    verify_config_my_private_id(&config_file.my_private_id)?;
    verify_config_shared_users(&config_file.shared_users)?;

    return Ok(());
}

fn verify_config_port(port: &String) -> Result<(), Box<dyn Error>> {
    let _port: i32 = match port.parse() {
        Ok(p) => p,
        Err(_) => Err("Port must be a number.")?,
    };
    return Ok(());
}

fn verify_config_my_private_id(my_private_id: &String) -> Result<(), Box<dyn Error>> {
    match crypto::get_bytes_from_base64_str(my_private_id) {
        Ok(_) => return Ok(()),
        Err(e) => Err(format!("Invalid Private ID. {}", e.to_string()))?,
    }
}

fn get_blocked_file_name_chars() -> String {
    let block_chars = String::from("/\\:*?\"<>|");
    return block_chars;
}

fn verify_config_shared_users(shared_users: &Vec<SharedUser>) -> Result<(), Box<dyn Error>> {
    // TODO
    // strip
    // not empty

    // Check duplicate names
    for user in shared_users {
        let mut user_count = 0;
        for userj in shared_users {
            if user.nickname == userj.nickname {
                user_count += 1;
            }
        }

        if user_count > 1 {
            return Err(format!("Nicknames cannot be repeated. '{}' was found '{}' times.", user.nickname, user_count))?;
        }
    }

    for user in shared_users {

        // Verify nickname
        if user.nickname == "" {
            Err("Nickname cannot be empty.")?;
        }
        for c in get_blocked_file_name_chars().chars() {
            if user.nickname.contains(c) {
                return Err(format!("Nickname '{}' contains the character '{}' which is not allowed. These characters are not allowed:   {}'", user.nickname, c, get_blocked_file_name_chars()))?;
            }
        }

        // Verify port
        if user.port == "" {
            Err(format!("Port for '{}' cannot be empty", user.nickname))?;
        }
        match verify_config_port(&user.port) {
            Ok(_) => {},
            Err(e) => Err(format!("{}'s port is invalid. {}", user.nickname, e.to_string()))?,
        }

        // Verify public id
        if user.public_id == "" {
            Err(format!("PublicID for '{}' cannot be empty", user.nickname))?;
        }
        let public_id = match crypto::get_bytes_from_base64_str(&user.public_id) {
            Ok(public_id) => public_id,
            Err(e) => Err(format!("{}'s PublicID is invalid. Bad encoding. {}", user.nickname, e.to_string()))?,
        };
        // TODO catch the panic on failure? There's no Result here???
        let parsed_id = signature::UnparsedPublicKey::new(&signature::ED25519, public_id);

        // Verify full ip and port address
        if user.ip == "" {
            Err(format!("IP Address for '{}' cannot be empty", user.nickname))?;
        }
        let full_address = format!("{}:{}", user.ip, user.port);
        let ip_parse: Result<SocketAddr, _> = full_address.parse();
        match ip_parse {
            Ok(_) => {},
            Err(e) => Err(format!("Full address of '{}', '{}' is not valid. Check IP and port. {}", user.nickname, full_address, e.to_string()))? ,
        }


    }
    
    return Ok(());
}

fn read_config() -> Result<ConfigFile, Box<dyn Error>> {
	let config_path = get_path_config_json()?;
	if !config_path.exists() {
        let exit_error = format!(
			"config.json does not exist at '{}'",
			config_path.to_string_lossy()
		);
        return Err(exit_error)?;
	}

	let config_string = fs::read_to_string(&config_path)?;
	let config_file: ConfigFile;
    match serde_json::from_str(&config_string.clone()) {
		Ok(c) => config_file = c,
		Err(e) => {
			let exit_error = format!(
				"config.json is invalid '{}' -- {}",
				config_path.to_string_lossy(), e.to_string()
			);
            return Err(exit_error)?;
		}
	};

    // TODO Sanitize the json. Trim all strings.

    verify_config(&config_file)?;

    return Ok(config_file);

}