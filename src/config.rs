use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::env;

extern crate base64;
use serde::{Deserialize, Serialize};

use crate::{
	crypto, 
	utils::{exit_error, get_blocked_display_name_chars, get_blocked_file_name_chars}
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
	pub my_private_id: String,
	pub trusted_users_public_ids: Vec<TrustedUser>,
	pub shared_files: Vec<ConfigSharedFile>,
	pub server_port: String,
}

impl Config {

	pub fn add_file(&mut self, new_files: Vec<ConfigSharedFile>) {
		for f in new_files {
				
			let mut skip = false;
			for c in self.shared_files.iter() {
				if c.path == f.path {
					skip = true;
				}
			}
			if skip {
				continue;
			}

			self.shared_files.push(f);
		}

		write_config(self);
	}

	pub fn remove_file(&mut self, file_path: String) -> Vec<String>{
		let mut shared_with_display_names = Vec::new();
		for f in self.shared_files.iter() {
			if f.path == file_path {
				shared_with_display_names = f.shared_with.clone();
				break;
			}
		}

		self.shared_files.retain(|x|*x.path != file_path);

		write_config(self);

		return shared_with_display_names;
	}

	pub fn remove_shared_with(&mut self, display_name: String, file_path: String) {
		for f in self.shared_files.iter_mut() {
			if f.path == file_path {
				f.shared_with.retain(|x|*x != display_name);
				break;
			}
		}

		write_config(self);
	}

	pub fn add_user_to_file(&mut self, display_name: &String, file_path: &String) {
		for file in self.shared_files.iter_mut() {
			if file.path == file_path.clone() {
				file.shared_with.push(display_name.clone());
				break;
			}
		}

		write_config(self);
	}

	pub fn remove_user(&mut self, display_name: &String) -> TrustedUser {
		let mut delete_index = 0;
		let mut cloned_user: Option<TrustedUser> = None;
		for user in self.trusted_users_public_ids.iter_mut() {
			if user.display_name == display_name.clone() {
				user.enabled = false;
				cloned_user = Some(user.clone());
				break;
			}
			delete_index += 1;
		}
		let found_cloned_user = cloned_user.unwrap();

		self.trusted_users_public_ids.remove(delete_index);

		
		// -- Remove user from shared files
		for files in self.shared_files.iter_mut() {
			files.shared_with.retain(|x| *x != display_name.clone());
		}

		write_config(self);

		return found_cloned_user;
	}

	pub fn disable_user(&mut self, display_name: &String) {
		for user in self.trusted_users_public_ids.iter_mut() {
			if user.display_name == display_name.clone() {
				user.enabled = false;
			}
		}
		write_config(self);
	}

	pub fn enable_user(&mut self, display_name: &String) {
		
		for user in self.trusted_users_public_ids.iter_mut() {
			if user.display_name == display_name.clone() {
				user.enabled = true;
			}
		}
		write_config(self);
	}

	pub fn edit_user(&mut self, current_display_name: &String, new_ip: &String, new_port: &String, new_public_id: &String) -> TrustedUser {
		let mut new_trusted_user: Option<TrustedUser> = None;
		for user in self.trusted_users_public_ids.iter_mut() {
			if current_display_name.clone() == user.display_name {
				user.ip_address = new_ip.clone();
				user.port = new_port.clone();
				user.public_id = new_public_id.clone();
				new_trusted_user = Some(user.clone());
				break;
			}
		}
		write_config(self);

		return new_trusted_user.unwrap();
	}

	pub fn process_new_user(&mut self, new_user: TrustedUser) {
		self.trusted_users_public_ids.push(new_user.clone());
		write_config(self);
	}

	pub fn get_port(&self) -> String {
		return self.server_port.clone();
	}
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigSharedFile {
	pub path: String,
	pub shared_with: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TrustedUser {
	pub public_id: String,
	pub display_name: String,
	pub ip_address: String,
	pub port: String,
	pub enabled: bool,
}


pub fn verify_config(config: &Config) {
	let blocked_file_name_chars = get_blocked_file_name_chars();
	let blocked_extended = get_blocked_display_name_chars();

	// public ids display name is valid
	for key in &config.trusted_users_public_ids {
		for c in blocked_extended.chars() {
			if key.display_name.contains(c) == true {
				exit_error(format!("public id display name '{}' from config.json contains char '{}'. The following are not allowed '{}'", key.display_name, c, blocked_extended));
			}
		}
	}

	// duplicate public id display names
	for key in &config.trusted_users_public_ids {
		let mut count = 0;
		for keyj in &config.trusted_users_public_ids {
			if keyj.display_name == key.display_name {
				count += 1;
			}
		}
		if count > 1 {
			exit_error(format!(
				"Public ID display name '{}' appears '{}' times. It can only be used once.",
				key.display_name, count
			));
		}
	}

	// shared file valid file names
	for f in &config.shared_files {
		for c in blocked_file_name_chars.chars() {
			if f.path.contains(c) == true {
				exit_error(format!("Shared file path '{}' from config.json contains char '{}'. The following are not allowed '{}'", f.path, c, blocked_file_name_chars));
			}
		}
	}

	// shared files exist
	for f in &config.shared_files {
		if Path::new(&f.path).exists() == false {
			exit_error(format!("Shared file in config doesn't exist: '{}'", f.path));
		}
	}

	// don't allow transmitic config
	let transmitic_path = get_path_transmitic_config_dir().to_str().unwrap().to_string();
	for f in &config.shared_files {
		if f.path.contains(&transmitic_path) {
			exit_error(format!("Cannot share the Transmitic configuration folder, or a file in it. {} ", f.path));
		}
	}

	// Shared With is valid
	let mut display_names: Vec<String> = Vec::new();
	for key in &config.trusted_users_public_ids {
		display_names.push(key.display_name.clone());
	}
	for f in &config.shared_files {
		for name in f.shared_with.iter() {
			if display_names.contains(name) == false {
				exit_error(format!(
					"Shared file '{}' is shared with '{}', which isn't found in the config.json",
					f.path, name
				));
			}
		}
	}
}


pub fn create_config_dir() {
	let path = get_path_transmitic_config_dir();
	println!("Transmitic Config Dir: {:?}", path);
	fs::create_dir_all(path).unwrap();
}

pub fn get_path_transmitic_config_dir() -> PathBuf {
	let mut path = env::current_exe().unwrap();
	path.pop();
	path.push("transmitic_config");
	return path;
}

pub fn get_path_downloads_dir() -> PathBuf {
	let mut path = get_path_transmitic_config_dir();
	path.push("downloads");
	return path;
}

pub fn get_path_downloads_dir_user(user: &String) -> PathBuf {
	let mut path = get_path_downloads_dir();
	path.push(user);
	return path;
}

pub fn get_path_download_queue(user: &TrustedUser) -> PathBuf {
	let mut path = get_path_transmitic_config_dir();
	path.push(format!("download_queue_{}.txt", user.display_name));
	return path;
}

pub fn delete_download_queue_file(user: &TrustedUser) {
	let file_path = get_path_download_queue(user);
	if file_path.exists() == true {
		fs::remove_file(file_path).unwrap();
	}
}

pub fn get_path_config_json() -> PathBuf {
	let mut path = get_path_transmitic_config_dir();
	path.push("transmitic_config.json");
	return path;
}

pub fn get_path_my_config_dir() -> PathBuf {
	let mut path = get_path_transmitic_config_dir();
	path.push("my_config");
	return path;
}

pub fn get_path_users_public_ids_dir() -> PathBuf {
	let mut path = get_path_transmitic_config_dir();
	path.push("users_public_ids");
	return path;
}


pub fn get_path_user_public_id_file(file_name: &String) -> PathBuf {
	let mut path = get_path_users_public_ids_dir();
	path.push(file_name);
	return path;
}

pub fn init_config() -> bool {
	let config_path = get_path_config_json();
	println!("config path: {:?}", config_path);

	if !config_path.exists() {
		create_new_config();
		return true;
	}

	return false;
}

pub fn create_new_config() {

	let (private_id_bytes, _) = crypto::generate_id_pair();

	let private_id_string = base64::encode(private_id_bytes);

	let empty_config: Config = Config{
	    my_private_id: private_id_string,
	    trusted_users_public_ids: Vec::new(),
	    shared_files: Vec::new(),
	    server_port: "7878".to_string(),
	};

	write_config(&empty_config);
}

pub fn write_config(config: &Config) {
	let config_path = get_path_config_json();
	let empty_config_str = serde_json::to_string(&config).unwrap();
	fs::write(config_path, empty_config_str).unwrap();
}

pub fn get_config() -> Config {
	let config_path = get_path_config_json();
	if config_path.exists() == false {
		exit_error(format!(
			"config.json does not exist at '{}'",
			config_path.to_str().unwrap()
		));
	}
	let config_string = fs::read_to_string(&config_path).unwrap();
	let config: Config = match serde_json::from_str(&config_string.clone()) {
		Ok(c) => c,
		Err(e) => {
			println!("{:?}", e);
			exit_error(format!(
				"config.json is invalid '{}'",
				config_path.to_str().unwrap()
			));
		}
	};
	return config;
}

