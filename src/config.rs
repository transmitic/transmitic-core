use std::env;
use std::error::Error;
use std::fs;
use std::fs::metadata;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

extern crate base64;
use ring::signature;
use ring::signature::Ed25519KeyPair;
use ring::signature::KeyPair;
use serde::{Deserialize, Serialize};

use crate::app_aggregator::AppAggMessage;
use crate::crypto;
use crate::shared_file::SharedFile;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigSharedFile {
    pub path: String,
    pub shared_with: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SharedUser {
    // Sanitize new fields!
    pub nickname: String,
    pub public_id: String,
    pub ip: String,
    pub port: String,
    pub allowed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ConfigFile {
    // Sanitize new fields!
    my_private_id: String,
    shared_users: Vec<SharedUser>,
    shared_files: Vec<ConfigSharedFile>,
    sharing_port: String,
}

#[derive(Clone)]
pub struct Config {
    first_start: bool,
    config_file: ConfigFile,
    local_private_key_bytes: Vec<u8>,
    path_dir_config: PathBuf,
}

// TODO Config is passed around and has methods that should NOT be allowed to be called everywhere
impl Config {
    pub fn new() -> Result<Config, Box<dyn Error>> {
        create_config_dir()?;
        let first_start = init_config()?;
        let config_file = read_config()?;
        let path_dir_config = get_path_transmitic_config_dir()?;
        let local_private_key_bytes =
            crypto::get_bytes_from_base64_str(&config_file.my_private_id)?;

        Ok(Config {
            first_start,
            config_file,
            local_private_key_bytes,
            path_dir_config,
        })
    }

    pub fn add_files(&mut self, files: Vec<String>) -> Result<(), Box<dyn Error>> {
        let mut new_config_file = self.config_file.clone();
        let mut existing_paths = Vec::new();
        for file in new_config_file.shared_files.iter() {
            existing_paths.push(file.path.clone());
        }

        for mut file in files {
            if file.starts_with("file://") {
                file = file[7..].to_string();
            }
            file = file.replace("/", "\\");
            // File already shared, don't readd it
            if existing_paths.contains(&file) {
                continue;
            }

            let shared_file = ConfigSharedFile {
                path: file,
                shared_with: Vec::new(),
            };
            new_config_file.shared_files.push(shared_file);
        }
        self.write_and_set_config(&mut new_config_file)?;

        Ok(())
    }

    pub fn add_new_user(
        &mut self,
        new_nickname: String,
        new_public_id: String,
        new_ip: String,
        new_port: String,
    ) -> Result<(), Box<dyn Error>> {
        let mut new_config_file = self.config_file.clone();
        let shared_user = SharedUser {
            public_id: new_public_id,
            nickname: new_nickname,
            ip: new_ip,
            port: new_port,
            allowed: true,
        };
        new_config_file.shared_users.push(shared_user);
        self.write_and_set_config(&mut new_config_file)?;

        Ok(())
    }

    pub fn add_user_to_shared(
        &mut self,
        nickname: String,
        file_path: String,
    ) -> Result<(), Box<dyn Error>> {
        let mut new_config_file = self.config_file.clone();

        // Is user valid
        let mut is_user_valid = false;
        for shared in &new_config_file.shared_users {
            if shared.nickname == nickname {
                is_user_valid = true;
                break;
            }
        }
        if !is_user_valid {
            return Err(format!(
                "Could not share file '{}'. User '{}' does not exist.",
                file_path, nickname
            )
            .into());
        }

        // Add user
        // TODO can I enumerate without ownership issues?
        for i in 0..new_config_file.shared_files.len() {
            if new_config_file.shared_files[i].path == file_path {
                if !new_config_file.shared_files[i]
                    .shared_with
                    .contains(&nickname)
                {
                    new_config_file.shared_files[i].shared_with.push(nickname);
                    self.write_and_set_config(&mut new_config_file)?;
                }
                return Ok(());
            }
        }
        return Err(format!(
            "Could not find file '{}' to share with user '{}'.",
            file_path, nickname
        )
        .into());
    }

    pub fn get_local_key_pair(&self) -> Ed25519KeyPair {
        let local_key_pair =
            signature::Ed25519KeyPair::from_pkcs8(self.local_private_key_bytes.as_ref()).unwrap();
        local_key_pair
    }

    pub fn get_local_private_id_bytes(&self) -> Vec<u8> {
        self.local_private_key_bytes.clone()
    }

    pub fn create_new_id(&mut self) -> Result<(), Box<dyn Error>> {
        let (private_id_bytes, _) = crypto::generate_id_pair()?;
        let private_id_string = base64::encode(private_id_bytes);

        let mut new_config_file = self.config_file.clone();
        new_config_file.my_private_id = private_id_string;
        self.write_and_set_config(&mut new_config_file)?;

        let local_private_key_bytes =
            crypto::get_bytes_from_base64_str(&self.config_file.my_private_id)?;
        self.local_private_key_bytes = local_private_key_bytes;

        Ok(())
    }

    pub fn get_path_dir_config(&self) -> PathBuf {
        self.path_dir_config.clone()
    }

    pub fn get_public_id_string(&self) -> String {
        let local_key_pair = self.get_local_key_pair();
        let public_id = local_key_pair.public_key().as_ref();

        crypto::get_base64_str_from_bytes(public_id.to_vec())
    }

    pub fn get_shared_files(&self) -> Vec<ConfigSharedFile> {
        self.config_file.shared_files.clone()
    }

    pub fn get_shared_users(&self) -> Vec<SharedUser> {
        self.config_file.shared_users.clone()
    }

    pub fn get_sharing_port(&self) -> String {
        self.config_file.sharing_port.clone()
    }

    pub fn is_first_start(&self) -> bool {
        self.first_start
    }

    pub fn remove_file_from_sharing(&mut self, file_path: String) -> Result<(), Box<dyn Error>> {
        let mut new_config_file = self.config_file.clone();
        new_config_file.shared_files.retain(|x| x.path != file_path);
        self.write_and_set_config(&mut new_config_file)?;

        Ok(())
    }

    pub fn remove_user_from_sharing(
        &mut self,
        nickname: String,
        file_path: String,
    ) -> Result<(), Box<dyn Error>> {
        let mut new_config_file = self.config_file.clone();

        for i in 0..new_config_file.shared_files.len() {
            if new_config_file.shared_files[i].path == file_path {
                new_config_file.shared_files[i]
                    .shared_with
                    .retain(|x| x != &nickname);
            }
        }

        self.write_and_set_config(&mut new_config_file)?;

        Ok(())
    }

    pub fn remove_user(&mut self, nickname: String) -> Result<(), Box<dyn Error>> {
        let mut new_config_file = self.config_file.clone();

        // Remove from shared files
        for i in 0..new_config_file.shared_files.len() {
            new_config_file.shared_files[i]
                .shared_with
                .retain(|x| x != &nickname);
        }

        // Remove from shared_users
        new_config_file
            .shared_users
            .retain(|x| x.nickname != nickname);
        self.write_and_set_config(&mut new_config_file)?;

        Ok(())
    }

    pub fn set_port(&mut self, port: String) -> Result<(), Box<dyn Error>> {
        let mut new_config_file = self.config_file.clone();
        new_config_file.sharing_port = port;
        self.write_and_set_config(&mut new_config_file)?;

        Ok(())
    }

    pub fn set_user_is_allowed_state(
        &mut self,
        nickname: String,
        is_allowed: bool,
    ) -> Result<(), Box<dyn Error>> {
        let mut new_config_file = self.config_file.clone();
        for user in new_config_file.shared_users.iter_mut() {
            if user.nickname == nickname {
                user.allowed = is_allowed;
                self.write_and_set_config(&mut new_config_file)?;
                return Ok(());
            }
        }

        return Err(format!(
            "Could not find user '{}' to set Allowed state '{}'.",
            nickname, is_allowed
        )
        .into());
    }

    pub fn update_user(
        &mut self,
        nickname: String,
        new_public_id: String,
        new_ip: String,
        new_port: String,
    ) -> Result<(), Box<dyn Error>> {
        let mut new_config_file = self.config_file.clone();
        for user in new_config_file.shared_users.iter_mut() {
            if user.nickname == nickname {
                user.public_id = new_public_id;
                user.ip = new_ip;
                user.port = new_port;

                self.write_and_set_config(&mut new_config_file)?;
                return Ok(());
            }
        }

        return Err(format!("Failed to find user '{}'. Could not update user.", nickname).into());
    }

    fn write_and_set_config(&mut self, config_file: &mut ConfigFile) -> Result<(), Box<dyn Error>> {
        write_config(config_file)?;
        self.config_file = config_file.to_owned();

        Ok(())
    }
}

fn create_config_dir() -> Result<(), std::io::Error> {
    let path = get_path_transmitic_config_dir()?;
    fs::create_dir_all(path)?;
    Ok(())
}

fn get_path_transmitic_config_dir() -> Result<PathBuf, std::io::Error> {
    let mut path = env::current_exe()?;
    path.pop();
    path.push("transmitic_config");
    Ok(path)
}

pub fn get_path_dir_downloads() -> Result<PathBuf, std::io::Error> {
    let mut path = env::current_exe()?;
    path.pop();
    path.push("Transmitic Downloads");
    Ok(path)
}

fn init_config() -> Result<bool, Box<dyn Error>> {
    let config_path = get_path_config_json()?;

    if !config_path.exists() {
        create_new_config()?;
        return Ok(true);
    }

    Ok(false)
}

fn get_path_config_json() -> Result<PathBuf, std::io::Error> {
    let mut path = get_path_transmitic_config_dir()?;
    path.push("transmitic_config.json");
    Ok(path)
}

fn create_new_config() -> Result<(), Box<dyn Error>> {
    let (private_id_bytes, _) = crypto::generate_id_pair()?;

    let private_id_string = base64::encode(private_id_bytes);

    let mut empty_config: ConfigFile = ConfigFile {
        my_private_id: private_id_string,
        shared_users: Vec::new(),
        shared_files: Vec::new(),
        sharing_port: "7878".to_string(),
    };

    write_config(&mut empty_config)?;
    Ok(())
}

fn write_config(config_file: &mut ConfigFile) -> Result<(), Box<dyn Error>> {
    verify_config(config_file)?;
    let config_path = get_path_config_json()?;
    let config_str = serde_json::to_string_pretty(config_file)?;
    fs::write(config_path, config_str)?;
    Ok(())
}

fn verify_config(config_file: &mut ConfigFile) -> Result<(), Box<dyn Error>> {
    sanitize_config(config_file);
    verify_config_port(&config_file.sharing_port)?;
    verify_config_my_private_id(&config_file.my_private_id)?;
    verify_config_shared_users(&config_file.shared_users)?;
    verify_config_shared_files(&config_file.shared_users, &config_file.shared_files)?;

    Ok(())
}

fn sanitize_config(config_file: &mut ConfigFile) {
    // DO A custom serde serializer that trims all strings?

    // trim
    config_file.my_private_id = config_file.my_private_id.trim().to_string();
    config_file.sharing_port = config_file.sharing_port.trim().to_string();

    // Sort shared users by nickname
    config_file.shared_users.sort_by_key(|x| x.nickname.clone());

    // Trim shared users strings
    for user in config_file.shared_users.iter_mut() {
        user.nickname = user.nickname.trim().to_string();
        user.public_id = user.public_id.trim().to_string();
        user.ip = user.ip.trim().to_string();
        user.port = user.port.trim().to_string();
    }

    // Trim shared files strings
    // remove file:// prefix
    // convert slashes
    for file in config_file.shared_files.iter_mut() {
        file.path = file.path.trim().to_string();

        if file.path.starts_with("file://") {
            file.path = file.path[7..].to_string();
        }

        file.path = file.path.replace("/", "\\");

        for i in 0..file.shared_with.len() {
            file.shared_with[i] = file.shared_with[i].trim().to_string();
        }
        file.shared_with.sort();
    }
}

fn verify_config_port(port: &str) -> Result<(), Box<dyn Error>> {
    let _port: i32 = match port.parse() {
        Ok(p) => p,
        Err(_) => return Err("Port must be a number.".into()),
    };
    Ok(())
}

fn verify_config_my_private_id(my_private_id: &str) -> Result<(), Box<dyn Error>> {
    let local_private_key_bytes = match crypto::get_bytes_from_base64_str(my_private_id) {
        Ok(local_private_key_bytes) => local_private_key_bytes,
        Err(e) => return Err(format!("Invalid Private ID. Not b64. {}", e.to_string()).into()),
    };

    match signature::Ed25519KeyPair::from_pkcs8(local_private_key_bytes.as_ref()) {
        Ok(key_pair) => key_pair,
        Err(e) => return Err(format!("Failed to load local key pair '{}'.", e.to_string()).into()),
    };

    Ok(())
}

fn get_blocked_file_name_chars() -> String {
    let mut block_chars = get_blocked_file_path_chars();
    block_chars.push('\\');
    block_chars.push(':');
    block_chars
}

fn get_blocked_file_path_chars() -> String {
    String::from("/*?\"<>|")
}

pub fn file_contains_only_valid_chars(path: &str) -> bool {
    for c in get_blocked_file_path_chars().chars() {
        if path.contains(c) {
            return false;
        }
    }

    // Remove .. paths, anything relative etc
    let path = Path::new(&path);
    for part in path.iter() {
        let mut keep = false;
        let s = part.to_str().unwrap().to_string();
        for c in s.chars() {
            if c != '.' {
                keep = true;
                break;
            }
        }

        if !keep {
            return false;
        }
    }

    true
}

fn verify_config_shared_users(shared_users: &[SharedUser]) -> Result<(), Box<dyn Error>> {
    // Check duplicate names and public ids
    for user in shared_users {
        let mut user_count = 0;
        let mut public_id_count = 0;
        for userj in shared_users {
            if user.nickname.to_lowercase() == userj.nickname.to_lowercase() {
                user_count += 1;
            }
            if user.public_id == userj.public_id {
                public_id_count += 1;
            }
        }

        if user_count > 1 {
            return Err(format!(
                "Nicknames cannot be repeated. '{}' was found '{}' times.",
                user.nickname, user_count
            )
            .into());
        }

        if public_id_count > 1 {
            return Err(format!(
                "Public IDs cannot be repeated. '{}' was found '{}' times.",
                user.public_id, public_id_count
            )
            .into());
        }
    }

    for user in shared_users {
        // Verify nickname
        if user.nickname.is_empty() {
            return Err("Nickname cannot be empty.".into());
        }
        for c in get_blocked_file_name_chars().chars() {
            if user.nickname.contains(c) {
                return Err(format!("Nickname '{}' contains the character '{}' which is not allowed. These characters are not allowed:   {}'.", user.nickname, c, get_blocked_file_name_chars()).into());
            }
        }

        // Verify port
        if user.port.is_empty() {
            return Err(format!("Port for '{}' cannot be empty", user.nickname).into());
        }
        match verify_config_port(&user.port) {
            Ok(_) => {}
            Err(e) => {
                return Err(
                    format!("{}'s port is invalid. {}", user.nickname, e.to_string()).into(),
                )
            }
        }

        // Verify public id
        if user.public_id.is_empty() {
            return Err(format!("PublicID for '{}' cannot be empty", user.nickname).into());
        }
        match crypto::get_bytes_from_base64_str(&user.public_id) {
            Ok(public_id) => public_id,
            Err(e) => {
                return Err(format!(
                    "{}'s PublicID is invalid. Bad encoding. {}",
                    user.nickname,
                    e.to_string()
                )
                .into())
            }
        };

        // Verify full ip and port address
        if user.ip.is_empty() {
            return Err(format!("IP Address for '{}' cannot be empty", user.nickname).into());
        }
        let full_address = format!("{}:{}", user.ip, user.port);
        let ip_parse: Result<SocketAddr, _> = full_address.parse();
        match ip_parse {
            Ok(_) => {}
            Err(e) => {
                return Err(format!(
                    "Full address of '{}', '{}' is not valid. Check IP and port. {}",
                    user.nickname,
                    full_address,
                    e.to_string()
                )
                .into())
            }
        }
    }

    Ok(())
}

fn verify_config_shared_files(
    shared_users: &[SharedUser],
    shared_files: &[ConfigSharedFile],
) -> Result<(), Box<dyn Error>> {
    // Check for duplicate files
    for file in shared_files {
        let mut file_count = 0;
        for filej in shared_files {
            if file.path == filej.path {
                file_count += 1;
            }
        }

        if file_count > 1 {
            return Err(format!(
                "File '{}' has been shared '{}' times. It can only be shared once.",
                file.path, file_count
            )
            .into());
        }
    }

    // Validate users
    let mut nicknames: Vec<String> = Vec::new();
    for user in shared_users {
        nicknames.push(user.nickname.clone());
    }

    for file in shared_files {
        for user in &file.shared_with {
            if !nicknames.contains(user) {
                return Err(format!("Cannot share file '{}' with user '{}' as that user does not exist as a shared_user.", file.path, user).into());
            }
        }
    }

    // Validate file paths
    let t = get_path_transmitic_config_dir()?;
    let t2 = t.as_os_str();
    let transmitic_config_dir_path = t2.to_str().ok_or(format!(
        "Failed to convert transmitic config dir path to os str. {:?}",
        t2
    ))?;
    for file in shared_files {
        if file.path.starts_with(transmitic_config_dir_path) {
            return Err(format!(
                "The Transmitic Config directory, and it's sub files, cannot be shared. '{}'",
                transmitic_config_dir_path
            )
            .into());
        }

        for c in get_blocked_file_path_chars().chars() {
            if file.path.contains(c) {
                return Err(format!("Cannot share file '{}' because it contains the character '{}' which is not allowed. These characters are not allowed:   {}'.", file.path, c, get_blocked_file_path_chars()).into());
            }
        }

        // Reject relative dirs
        if !file_contains_only_valid_chars(&file.path) {
            return Err(format!(
                "Cannot share file with relative directories or invalid characters. {}",
                file.path
            )
            .into());
        }
    }

    Ok(())
}

fn read_config() -> Result<ConfigFile, Box<dyn Error>> {
    let config_path = get_path_config_json()?;
    if !config_path.exists() {
        let exit_error = format!(
            "config.json does not exist at '{}'",
            config_path.to_string_lossy()
        );
        return Err(exit_error.into());
    }

    let config_string = fs::read_to_string(&config_path)?;
    let mut config_file: ConfigFile;
    match serde_json::from_str(&config_string) {
        Ok(c) => config_file = c,
        Err(e) => {
            let exit_error = format!(
                "config.json is invalid '{}' -- {}",
                config_path.to_string_lossy(),
                e.to_string()
            );
            return Err(exit_error.into());
        }
    };

    verify_config(&mut config_file)?;

    Ok(config_file)
}

// TODO move into config?
pub fn get_everything_file(
    app_sender: &Sender<AppAggMessage>,
    config: &Config,
    nickname: &str,
) -> Result<SharedFile, Box<dyn Error>> {
    // The "root" everything directory
    let mut everything_file = SharedFile::new("everything/".to_string(), true, Vec::new(), 0);

    // Get SharedFiles
    for file in &config.get_shared_files() {
        if !file.shared_with.contains(&nickname.to_string()) {
            continue;
        }

        let path: String = file.path.clone();
        // Check existence. Skip if missing.
        // TODO make an app error so it's known the file is missing for transfers
        let meta_data = match metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                app_sender.send(AppAggMessage::LogError(format!(
                    "Path unable to share. Fix issue or stop sharing file. '{}'. {}",
                    path,
                    e.to_string()
                )))?;
                continue;
            }
        };

        let is_directory = meta_data.is_dir();

        let mut file_size: u64 = 0; // directory size calculated by all files
        if !is_directory {
            file_size = metadata(&path)?.len();
        }

        let mut shared_file: SharedFile =
            SharedFile::new(path, is_directory, Vec::new(), file_size);
        let config_dir = match &config.get_path_dir_config().to_str() {
            Some(config_dir) => config_dir.to_string(),
            None => return Err("Failed to convert config dir path to String".into()),
        };
        process_shared_file(&mut shared_file, &config_dir)?;
        everything_file.file_size += shared_file.file_size;
        everything_file.files.push(shared_file);
    }

    everything_file.set_file_size_string();

    Ok(everything_file)
}

pub fn process_shared_file(
    shared_file: &mut SharedFile,
    config_dir: &str,
) -> Result<(), Box<dyn Error>> {
    if !shared_file.is_directory {
        return Ok(());
    }

    if shared_file.is_directory {
        for file in fs::read_dir(&shared_file.path)? {
            let file = file?;
            let path = file.path();

            let path_string = match path.to_str() {
                Some(path_string) => String::from(path_string),
                None => return Err(format!("Failed to convert path string {:?}", path).into()),
            };

            if path_string.contains(config_dir) {
                continue;
            }

            let is_directory = path.is_dir();

            let mut file_size: u64 = 0; // directory size calculated by all files
            if !is_directory {
                file_size = metadata(&path_string)?.len();
            }
            let mut new_shared_file =
                SharedFile::new(path_string, is_directory, Vec::new(), file_size);
            if is_directory {
                process_shared_file(&mut new_shared_file, config_dir)?;
            }
            shared_file.file_size += new_shared_file.file_size;
            shared_file.set_file_size_string();
            shared_file.files.push(new_shared_file);
        }
    }

    Ok(())
}
