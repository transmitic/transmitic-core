use std::{
	net::SocketAddr, 
	sync::{Arc, Mutex}, 
	thread
};

extern crate x25519_dalek;
use ring::{
	signature::{self, KeyPair},
};

use crate::{
	config::{self, Config, ConfigSharedFile, TrustedUser, create_config_dir, get_config, init_config, verify_config}, 
	crypto, 
	incoming::{IncomingConnection, IncomingConnectionManager, client_wait_for_incoming}, 
	outgoing::{OutgoingConnection, OutgoingConnectionManager, handle_outgoing_forever}, 
	shared_file::FileToDownload, 
	utils::{LocalKeyData, get_blocked_display_name_chars, get_blocked_file_name_chars}
};

pub struct ReturnCurrentUser {
	pub display_name: String,
	pub enabled: bool,
	pub ip_address: String,
	pub port: String,
	pub public_id: String,
}

pub struct ReturnUserDownloadingFromMe {
	pub display_name: String,
	pub file_path: String,
	pub download_percent: usize,
}

pub struct TransmiticCore {
	pub config: Arc<Mutex<Config>>, // TODO make private
	pub local_key_data: Arc<Mutex<LocalKeyData>>,
	pub outgoing_connection_manager: OutgoingConnectionManager,  // TODO make this private again?
	incoming_connections: IncomingConnectionManager,
	is_first_start: bool,
	sharing_mode: Arc<Mutex<String>>
}

impl TransmiticCore {
    pub fn new() -> TransmiticCore {
        create_config_dir();
        let first_load = init_config();
        let config = get_config();
        verify_config(&config);

        // Load local private key pair
        let local_private_key_bytes = crypto::get_bytes_from_base64_str(&config.my_private_id);
        let local_key_pair =
            signature::Ed25519KeyPair::from_pkcs8(local_private_key_bytes.as_ref()).unwrap();
        
        let local_key_data = LocalKeyData {
            local_key_pair: local_key_pair,
            local_key_pair_bytes: local_private_key_bytes,
        };
        let local_key_data_arc = Arc::new(Mutex::new(local_key_data));

        let config_arc = Arc::new(Mutex::new(config));

        TransmiticCore {
            config: Arc::clone(&config_arc),
            local_key_data: Arc::clone(&local_key_data_arc),
            outgoing_connection_manager: OutgoingConnectionManager::new(
                Arc::clone(&config_arc),
                Arc::clone(&local_key_data_arc),
                false,
            ),
            incoming_connections: IncomingConnectionManager::new(
                Arc::clone(&config_arc),
            ),
            is_first_start: first_load,
            sharing_mode: Arc::new(Mutex::new(String::from("Off"))),
        }
    }

    pub fn client_start_sharing(&mut self) {
        let config_clone = Arc::clone(&self.config);
		let local_key_data_clone = Arc::clone(&self.local_key_data);
		let incoming_clone = Arc::clone(&self.incoming_connections.incoming_connections);
		let sharing_clone = Arc::clone(&self.sharing_mode);
		thread::spawn(move || {
			client_wait_for_incoming(
				incoming_clone,
				config_clone,
				local_key_data_clone,
				sharing_clone,
			);
		});
    }

    pub fn download_file_list(&mut self, files_to_download: Vec<FileToDownload>) {
        self.outgoing_connection_manager
			.download_files(files_to_download);
    }

    pub fn set_sharing_mode(&mut self, sharing_mode: String) {
        let mut sharing_guard = self.sharing_mode			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		sharing_guard.clear();
		sharing_guard.push_str(&sharing_mode);

		std::mem::drop(sharing_guard);

		self.incoming_connections.reset_all_connections_for_all_users();
    }

    pub fn remove_file(&mut self, file_path: String) {
        let mut config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		let shared_with_display_names = config_guard.remove_file(file_path.clone());
		std::mem::drop(config_guard);

		self.incoming_connections.reset_all_connections_for_specific_users(&shared_with_display_names);
    }

    pub fn remove_shared_with(&mut self, display_name: &String, file_path: String) {
        
		let mut config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		config_guard.remove_shared_with(display_name.clone(), file_path);
		std::mem::drop(config_guard);

		self.incoming_connections.reset_all_connections(&display_name);
    }

    pub fn remove_user(&mut self, display_name: &String) {
        let mut config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		let found_cloned_user = config_guard.remove_user(&display_name);
		std::mem::drop(config_guard);

		self.incoming_connections.remove_user(&display_name);

		config::delete_download_queue_file(&found_cloned_user);

		let mut outgoing_connection = self.outgoing_connection_manager.outgoing_connections.get(display_name).unwrap()
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		outgoing_connection.is_deleted = true;
		std::mem::drop(outgoing_connection);

		self.outgoing_connection_manager.outgoing_connections.remove(display_name);
    }

	pub fn add_user_to_file(&mut self, display_name: &String, file_path: &String) {
		// TODO verify dispaly_name is valid: allowed and exists in keys
		let mut config_guard = self.config
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		config_guard.add_user_to_file(&display_name, &file_path);
		std::mem::drop(config_guard);

		self.incoming_connections.reset_all_connections(&display_name);
	}

	pub fn disable_user(&mut self, display_name: &String) {
		let mut config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		
		config_guard.disable_user(&display_name);

		std::mem::drop(config_guard);

		self.incoming_connections.disable_user(&display_name);
	}

	pub fn enable_user(&mut self, display_name: &String) {
		let mut config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		config_guard.enable_user(&display_name);
		std::mem::drop(config_guard);

		self.incoming_connections.enable_user(&display_name);
	}

	pub fn process_new_user(&mut self, display_name: &String, public_id: &String, ip_address: &String, port: &String) {
		let new_user: config::TrustedUser = TrustedUser {
			public_id: public_id.clone(),
			display_name: display_name.clone(),
			ip_address: ip_address.clone(),
			port: port.clone(),
			enabled: true,
		};
		let mut config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		config_guard.process_new_user(new_user.clone());
		std::mem::drop(config_guard);

		// -- Add incoming connection
		// TODO dupe
		let incoming_connection = IncomingConnection {
			user: new_user.clone(),
			active_download: None,
			active_download_percent: 0,
			active_download_current_bytes: 0.0,
			is_downloading: false,
			finished_downloads: Vec::new(),
			is_disabled: false,
			//should_reset_connection: false,
			single_connections: Vec::new(),
		};
		let incomig_mutex = Arc::new(Mutex::new(incoming_connection));
		let mut incoming_connections_guard = self.incoming_connections.incoming_connections
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		incoming_connections_guard.insert(display_name.clone(), incomig_mutex);
		std::mem::drop(incoming_connections_guard);
		
		// -- Add outgoing connection
		// TODO dupe
		let outgoing_connection = OutgoingConnection::new(new_user.clone(), self.outgoing_connection_manager.is_all_paused);
		let outgoing_connection_arc = Arc::new(Mutex::new(outgoing_connection));
		let outgoing_connection_arc_clone = Arc::clone(&outgoing_connection_arc);
		let outgoing_connection_arc_clone2 = Arc::clone(&outgoing_connection_arc);
		self.outgoing_connection_manager.outgoing_connections.insert(new_user.display_name.clone(), outgoing_connection_arc_clone2);
		
		let local_key_data_arc = Arc::clone(&self.local_key_data);

		let thread_name = display_name.clone();
		thread::spawn(move || {
			handle_outgoing_forever(&outgoing_connection_arc_clone, local_key_data_arc);
			println!("Outgoing final exit {}", thread_name);
		});
	}

	pub fn create_new_id(&mut self) -> String {
		let (private_id_bytes, public_id_bytes) = crypto::generate_id_pair();

		let private_id_string = base64::encode(&private_id_bytes);
		let public_id_string = base64::encode(&public_id_bytes);

		// TODO config_guard
		let mut config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		config_guard.my_private_id = private_id_string;

		let mut local_key_data_guard = self.local_key_data
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		local_key_data_guard.local_key_pair = signature::Ed25519KeyPair::from_pkcs8(private_id_bytes.as_ref()).unwrap();
		local_key_data_guard.local_key_pair_bytes = private_id_bytes;
		std::mem::drop(local_key_data_guard);

		config::write_config(&config_guard);
		std::mem::drop(config_guard);
		
		self.incoming_connections.reset_all_connections_for_all_users();

		for (_, outgoing_connection) in self.outgoing_connection_manager.outgoing_connections.iter_mut() {
			let mut outgoing_connection_guard = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
			outgoing_connection_guard.should_reset_connection = true;
			std::mem::drop(outgoing_connection_guard);
		}

		return public_id_string;
	}

	pub fn get_port(&self) -> String {
		let config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		let port = config_guard.get_port();
		std::mem::drop(config_guard);
		return port;
	}

	pub fn get_sharing_mode(&self) -> String {
		let sharing_guard = self.sharing_mode	
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		let sharing_mode = sharing_guard.clone();
		std::mem::drop(sharing_guard);

		return sharing_mode;
	}

	pub fn get_public_id(&self) -> String {
		let local_key_data_guard = self.local_key_data
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		let public_id = local_key_data_guard.local_key_pair.public_key().as_ref();
		let s = crypto::get_base64_str_from_bytes(public_id.to_vec());
		std::mem::drop(local_key_data_guard);

		return s;
	}

	pub fn get_is_first_start(&self) -> bool {
		return self.is_first_start;
	}

	pub fn get_current_users(&self) -> Vec<ReturnCurrentUser> {
		let mut users: Vec<ReturnCurrentUser> = Vec::new();

		// TODO config_guard
		let config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		for user in config_guard.trusted_users_public_ids.iter() {
			let current_user = ReturnCurrentUser {
				display_name: user.display_name.clone(),
				enabled: user.enabled.clone(),
				ip_address: user.ip_address.clone(),
				port: user.port.clone(),
				public_id: user.public_id.clone(),
			};
			users.push(current_user);
		}
		std::mem::drop(config_guard);

		return users;
	}

	pub fn get_downloading_from_me(&self) -> (Vec<ReturnUserDownloadingFromMe>, Vec<ReturnUserDownloadingFromMe>) {
		let mut active_downloading: Vec<ReturnUserDownloadingFromMe> = Vec::new();
		let mut finished_downloading: Vec<ReturnUserDownloadingFromMe> = Vec::new();

		let incoming_connections_guard = self.incoming_connections.incoming_connections
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		for (user_name, connection) in incoming_connections_guard.iter() {
			let conn = connection
				.lock()
				.unwrap_or_else(|poisoned| poisoned.into_inner());

			if conn.is_downloading {
				let download_user = ReturnUserDownloadingFromMe {
					display_name: user_name.clone(),
					file_path: conn.active_download.clone().unwrap().path.clone(),
					download_percent: conn.active_download_percent.clone(),
				};
				active_downloading.push(download_user);
			}

			for path in conn.finished_downloads.iter() {
				let finished_user = ReturnUserDownloadingFromMe {
					display_name: user_name.clone(),
					file_path: path.clone(),
					download_percent: 100,
				};
				finished_downloading.push(finished_user);
			}
			std::mem::drop(conn);
		}

		std::mem::drop(incoming_connections_guard);

		return (active_downloading, finished_downloading);
	}

	pub fn get_my_shared_files(&self) -> (Vec<ConfigSharedFile>, Vec<String>){
		let config_guard = self.config
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		let mut my_shared_files: Vec<ConfigSharedFile> = Vec::new();
		let mut users: Vec<String> = Vec::new();

		for user in config_guard.trusted_users_public_ids.iter() {
			users.push(user.display_name.to_string());
		}

		for file in config_guard.shared_files.iter() {
			my_shared_files.push(file.clone());
		}

		std::mem::drop(config_guard);

		return (my_shared_files, users);
	}

	pub fn get_user_names(&self) -> Vec<String> {
		let mut user_list: Vec<String> = Vec::new();
		
		let config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		for user in config_guard.trusted_users_public_ids.iter() {
			user_list.push(user.display_name.clone());
		}
		std::mem::drop(config_guard);

		return user_list;
	}

	pub fn add_files(&mut self, file_paths: &Vec<String>) -> Result<String, String> {

		let transmitic_path = config::get_path_transmitic_config_dir().to_str().unwrap().to_string();
		let mut new_files = Vec::new();
		for file_path in file_paths.into_iter() {
			
			if file_path.contains(&transmitic_path) {
				return Err(format!("No files added. Cannot share the Transmitic configuration folder, or a file in it. {} ", transmitic_path));
			}
			
			let blocked_file_name_chars = get_blocked_file_name_chars();
			for c in blocked_file_name_chars.chars() {
				if file_path.contains(c) == true {
					return Err(format!("No files added. Cannot share, '{}', since it contains the character: {} . The following are not allowed: {}", file_path, c, blocked_file_name_chars));
				}
			}

			let new_shared_file = config::ConfigSharedFile {
				path: file_path.clone(),
				shared_with: Vec::new(),
			};

			new_files.push(new_shared_file);
		}


		let mut config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		config_guard.add_file(new_files);
		std::mem::drop(config_guard);

		return Ok("Files have been added".to_string());
	}

	pub fn add_new_user(&mut self, display_name: &String, public_id: &String, ip_address: &String, port: &String) -> Result<String, String> {
		
		// Initial check
		let (code, message ) = self.verify_inital_new_user_data(display_name, public_id, ip_address, port);
		if code != 0 {
			return  Err(message);
		}

		// -- Look for duplicates
		// TODO config_guard
		let config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		let mut response_code = 0;
		let mut response_msg: String = String::from("");
		for user in config_guard.trusted_users_public_ids.iter() {
			if display_name.clone() == user.display_name {
				response_code = 1;
				response_msg = "Nickname already in use.".to_string();
				break;
			}

			if public_id.clone() == user.public_id {
				response_code = 1;
				response_msg = format!("Public ID already in use by {}", user.display_name);
				break;
			}

			if ip_address.clone() == user.ip_address {
				response_code = 1;
				response_msg = format!("IP Address already in use by {}", user.display_name);
				break;
			}
		}
		std::mem::drop(config_guard);

		if response_code == 0 {
			self.process_new_user(display_name, public_id, ip_address, port);
			response_msg = format!("User {} successfully added", display_name);
			return Ok(response_msg);
		} else {
			return Err(response_msg);
		}
	}

	pub fn edit_user(&mut self, display_name: &String, new_public_id: &String, new_ip: &String, new_port: &String) -> Result<String, String> {
		// Initial check
		let (code, message ) = self.verify_inital_new_user_data(&display_name, &new_public_id, &new_ip, &new_port);
		if code != 0 {
			return  Err(message);
		}

		// -- Look for duplicates
		// TODO config_guard
		let mut config_guard = self.config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		let mut response_code = 0;
		let mut response_msg: String = String::from("");
		for user in config_guard.trusted_users_public_ids.iter() {
			if display_name.clone() == user.display_name {
				continue;
			}

			if new_public_id.clone() == user.public_id {
				response_code = 1;
				response_msg = format!("Public ID already in use by {}", user.display_name);
				break;
			}

			if new_ip.clone() == user.ip_address {
				response_code = 1;
				response_msg = format!("IP Address already in use by {}", user.display_name);
				break;
			}
		}

		if response_code != 0 {
			std::mem::drop(config_guard);
			return Err(response_msg);
		}

		/*
		- resets all connections
			- new id
			- local/internet
		*/
		// Modify existing user
		let new_trusted_user = config_guard.edit_user(&display_name, &new_ip, &new_port, &new_public_id);
		response_msg = format!("User '{}' edited successfully", new_trusted_user.display_name);

		let incoming_connections_guard = self.incoming_connections.incoming_connections
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		let incoming_connection = incoming_connections_guard
		.get(display_name)
		.unwrap();

		let mut connection_guard = incoming_connection
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		
		let mut outgoing_connection = self.outgoing_connection_manager.outgoing_connections.get(display_name).unwrap()
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		connection_guard.user = new_trusted_user.clone();
		outgoing_connection.user = new_trusted_user.clone();
		outgoing_connection.should_reset_connection = true;
		
		std::mem::drop(config_guard);
		std::mem::drop(connection_guard);
		std::mem::drop(incoming_connections_guard);
		std::mem::drop(outgoing_connection);

		return Ok(response_msg);
	}

	fn verify_inital_new_user_data(&self, display_name: &String, public_id: &String, ip_address: &String, port: &String) -> (i32, std::string::String) {
		// -- Display name
		// Chars
		if display_name.len() == 0 {
			return (1, "Nickname cannot be empty".to_string());
		}
		let blocked_chars = get_blocked_display_name_chars();
		for c in display_name.chars() {
			if blocked_chars.contains(c) {
				let msg = format!("Nickname contains disallowed letter '{}'. These letters are not allowed: {}", c, blocked_chars);
				return (1, msg);
			}
		}

		// -- Public ID
		// Chars
		if public_id.len() == 0 {
			return (1, "Public ID cannot be empty".to_string());
		}
		// Valid Parse
		match base64::decode(&public_id) {
			Ok(_) => {},
			Err(e) => {
				let msg = format!("Invalid Public ID. Failed to decode. {}", e);
				return (1, msg);
			}
		}
		
		// -- Port
		// Chars
		if port.len() == 0 {
			return (1, "Port cannot be empty".to_string());
		}
		
		// -- IP Address
		// Chars
		if ip_address.len() == 0 {
			return (1, "IP Address cannot be empty".to_string());
		}
		// Valid Parse
		let ip_combo = format!("{}:{}", ip_address, port);
		let ip_parse: Result<SocketAddr, _> = ip_combo.parse();
		match ip_parse {
			Ok(_) => {},
			Err(e) => {
				let msg = format!("IP Address and port, {}, is not valid: {}", ip_combo, e);
				return (1, msg);
			}
		}

		return (0, "".to_string());
	}
}