use core::time;
use std::fs;
use std::fs::metadata;
use std::fs::File;
use std::io::prelude::*;
use std::{
	collections::{HashMap, VecDeque}, 
	fs::OpenOptions, 
	io::SeekFrom, 
	net::TcpStream, 
	panic, 
	path::Path, 
	sync::{Arc, Mutex}, 
	thread
};
use std::net::SocketAddr;

use crate::{
	config::{self, Config, TrustedUser}, 
	core_consts::{MSG_CANNOT_SELECT_DIRECTORY, MSG_CLIENT_DIFFIE_PUBLIC, MSG_FILE_CHUNK, MSG_FILE_FINISHED, MSG_FILE_INVALID_FILE, MSG_FILE_SELECTION_CONTINUE, PAYLOAD_OFFSET, TOTAL_BUFFER_SIZE}, 
	crypto, 
	secure_stream::SecureStream, 
	shared_file::{FileToDownload, SharedFile}, 
	utils::{LocalKeyData, get_file_by_path, get_payload_size_from_buffer, get_size_of_directory, request_file_list, set_buffer}
};

use aes_gcm::aead::{generic_array::GenericArray, NewAead};
use aes_gcm::Aes256Gcm;
use rand_core::OsRng;
use ring::signature;
use x25519_dalek::EphemeralSecret;
use x25519_dalek::PublicKey;

pub struct OutgoingConnectionManager {
	pub config: Arc<Mutex<Config>>,
	pub outgoing_connections: HashMap<String, Arc<Mutex<OutgoingConnection>>>,
	pub is_all_paused: bool,
}

impl OutgoingConnectionManager {
	pub fn new(config: Arc<Mutex<Config>>, local_key_data: Arc<Mutex<LocalKeyData>>, is_all_paused: bool) -> OutgoingConnectionManager {
		let mut connections: HashMap<String, Arc<Mutex<OutgoingConnection>>> = HashMap::new();
		
		let config_guard = config			
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		for user in config_guard.trusted_users_public_ids.iter() {
			let outgoing_connection = OutgoingConnection::new(user.clone(), is_all_paused);
			let outgoing_connection_arc = Arc::new(Mutex::new(outgoing_connection));
			let outgoing_connection_arc_clone = Arc::clone(&outgoing_connection_arc);
			let outgoing_connection_arc_clone2 = Arc::clone(&outgoing_connection_arc);
			connections.insert(user.display_name.clone(), outgoing_connection_arc_clone2);
			let thread_name = user.display_name.clone();
			
			let local_key_data_arc = Arc::clone(&local_key_data);

			thread::spawn(move || {
				handle_outgoing_forever(&outgoing_connection_arc_clone, local_key_data_arc);
				println!("Outgoing final exit {}", thread_name);
			});
		}
		std::mem::drop(config_guard);

		OutgoingConnectionManager {
			config: config,
			outgoing_connections: connections,
			is_all_paused: is_all_paused,
		}
	}

	pub fn download_files(&mut self, file_list: Vec<FileToDownload>) {
		for f in file_list {
			let owner = f.file_owner;
			let path = f.file_path;
			let mut conn = self
				.outgoing_connections
				.get(&owner)
				.unwrap()
				.lock()
				.unwrap_or_else(|poisoned| poisoned.into_inner());
			conn.download_file(path);
		}
	}

	pub fn cancel_all_downloads(&mut self) {
		for (_, outgoing_connection) in self.outgoing_connections.iter() {
			let mut outgoing_connection_guard = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned|poisoned.into_inner());

			match &outgoing_connection_guard.active_download {
				Some(_) => {
					//outgoing_connection_guard.active_download = None
					outgoing_connection_guard.stop_active_download = true;
					
				}
				_ => {
				}
			}
			outgoing_connection_guard.download_queue.clear();
			std::mem::drop(outgoing_connection_guard);

		}
	}

	pub fn cancel_single_download(&mut self, display_name: &String, file_path: &String) {
		let outgoing_connection = self.outgoing_connections.get(display_name).unwrap();
		let mut outgoing_connection_guard = outgoing_connection
		.lock()
		.unwrap_or_else(|poisoned|poisoned.into_inner());

		match &outgoing_connection_guard.active_download {
			Some(f) => {
				if f.path == file_path.replace("\\\\", "\\") {
					outgoing_connection_guard.stop_active_download = true;
					outgoing_connection_guard.active_download = None; // TODO is this needed?
				} else {
					outgoing_connection_guard.download_queue.retain(|x|x != file_path);
					outgoing_connection_guard.write_queue();
				}
			}
			_ => {
				outgoing_connection_guard.download_queue.retain(|x|x != &file_path.replace("\\\\", "\\"));
				outgoing_connection_guard.write_queue();
			}
		}
		std::mem::drop(outgoing_connection_guard);
	}

	pub fn resume_downloads(&mut self, display_name: &String) {
		let outgoing_connection = self.outgoing_connections.get(display_name).unwrap();
		let mut outgoing_connection_guard = outgoing_connection
		.lock()
		.unwrap_or_else(|poisoned|poisoned.into_inner());

		outgoing_connection_guard.is_paused = false;

		std::mem::drop(outgoing_connection_guard);
	}

	pub fn resume_all_downloads(&mut self) {
		for (_, outgoing_connection) in self.outgoing_connections.iter() {
			let mut outgoing_connection_guard = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned|poisoned.into_inner());

			outgoing_connection_guard.is_paused = false;

			std::mem::drop(outgoing_connection_guard);

		}

		self.is_all_paused = false;
	}

	pub fn pause_all_downloads(&mut self) {
		self.is_all_paused = true;
		for (_, outgoing_connection) in self.outgoing_connections.iter() {
			let mut outgoing_connection_guard = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned|poisoned.into_inner());

			outgoing_connection_guard.is_paused = true;
			outgoing_connection_guard.stop_active_download = true;

			std::mem::drop(outgoing_connection_guard);
		}
	}

	pub fn pause_downloads_for_user(&mut self, display_name: &String) {
		let outgoing_connection = self.outgoing_connections.get(display_name).unwrap();
		let mut outgoing_connection_guard = outgoing_connection
		.lock()
		.unwrap_or_else(|poisoned|poisoned.into_inner());

		outgoing_connection_guard.is_paused = true;
		outgoing_connection_guard.stop_active_download = true;

		std::mem::drop(outgoing_connection_guard);
	}

	pub fn clear_invalid_downloads(&mut self) {
		for (_, outgoing_connection) in self.outgoing_connections.iter() {
			let mut outgoing_connection_guard = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned|poisoned.into_inner());

			outgoing_connection_guard.invalid_downloads.clear();

			std::mem::drop(outgoing_connection_guard);

		}
	}

	pub fn clear_finished_downloads(&mut self) {
		for (_, outgoing_connection) in self.outgoing_connections.iter() {
			let mut outgoing_connection_guard = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned|poisoned.into_inner());

			outgoing_connection_guard.finished_downloads.clear();

			std::mem::drop(outgoing_connection_guard);

		}
	}
}

pub fn handle_outgoing_forever(
	outgoing_connection: &Arc<Mutex<OutgoingConnection>>,
	local_key_data: Arc<Mutex<LocalKeyData>>,
) {
	loop {
		let mut connection_guard = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		if connection_guard.is_deleted == true {
			println!("Outgoing Forever Connection deleted. {}", connection_guard.user.display_name);
			std::mem::drop(connection_guard);
			break;
		}
		if connection_guard.download_queue.is_empty() {
			std::mem::drop(connection_guard);
			thread::sleep(time::Duration::from_millis(1000));
			continue;
		}
		connection_guard.should_reset_connection = false;
		std::mem::drop(connection_guard);
		
		let local_key_data_guard = local_key_data
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		let local_key_pair_bytes_clone = local_key_data_guard.local_key_pair_bytes.clone();
		std::mem::drop(local_key_data_guard);

		let _ = panic::catch_unwind(|| {
			handle_outgoing(outgoing_connection, &local_key_pair_bytes_clone);
		});
		std::mem::drop(outgoing_connection);
		let mut connection_guard = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		connection_guard.is_online = false;
		if connection_guard.is_deleted == true {
			println!("Connection deleted. {}", connection_guard.user.display_name);
			std::mem::drop(connection_guard);
			break;
		}
		std::mem::drop(connection_guard);
		println!("Error. Reconnecting...");
		thread::sleep(time::Duration::from_millis(10000));
	}
}

pub struct OutgoingConnection {
	pub user: TrustedUser,
	pub download_queue: VecDeque<String>,
	pub finished_downloads: Vec<(String, String)>,
	pub invalid_downloads: Vec<String>,
	pub root_file: Option<SharedFile>,
	pub active_download: Option<SharedFile>,
	pub active_download_percent: usize,
	pub active_download_current_bytes: f64,
	pub is_online: bool,
	pub is_deleted: bool,
	pub should_reset_connection: bool,
	pub stop_active_download: bool,
	pub is_paused: bool,
}

impl OutgoingConnection {
	pub fn new(user: TrustedUser, is_all_paused: bool) -> OutgoingConnection {
		let queue_path = config::get_path_download_queue(&user);
		let mut download_queue: VecDeque<String> = VecDeque::new();
		if queue_path.exists() {
			let f = fs::read_to_string(queue_path).expect("Unable to read file");
			for mut line in f.lines() {
				line = line.trim();
				if line != "" {
					download_queue.push_back(line.to_string());
				}
			}
		}

		OutgoingConnection {
			user: user,
			download_queue: download_queue,
			finished_downloads: Vec::new(),
			invalid_downloads: Vec::new(),
			root_file: None,
			active_download: None,
			active_download_percent: 0,
			active_download_current_bytes: 0.0,
			is_online: false,
			is_deleted: false,
			should_reset_connection: false,
			stop_active_download: false,
			is_paused: is_all_paused,
		}
	}


	pub fn download_file(&mut self, file_path: String) {
		println!(
			"DOWNLOAD FILE FOR: {} - {}",
			self.user.display_name, file_path
		);

		&self.download_queue.push_back(file_path);
		self.write_queue();
	}

	pub fn write_queue(&self) {
		let mut write_string = String::new();

		if let Some(shared_file) = &self.active_download {
			write_string.push_str(&format!("{}\n", &shared_file.path));
		}

		for f in &self.download_queue {
			write_string.push_str(&format!("{}\n", &f));
		}

		let file_path = config::get_path_download_queue(&self.user);
		let mut f = OpenOptions::new()
			.write(true)
			.create(true)
			.truncate(true)
			.open(file_path)
			.unwrap();
		f.write(write_string.as_bytes()).unwrap();
	}
}

fn handle_outgoing(
	outgoing_connection: &Arc<Mutex<OutgoingConnection>>,
	local_key_pair_bytes: &Vec<u8>,
) {
	let mut connection_guard = outgoing_connection
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	let local_key_pair =
		signature::Ed25519KeyPair::from_pkcs8(local_key_pair_bytes.as_ref()).unwrap();
	let mut remote_addr = String::from(&connection_guard.user.ip_address);
	remote_addr.push_str(&format!(":{}", &connection_guard.user.port));

	println!(
		"Handle Outgoing {} - {}",
		remote_addr, &connection_guard.user.display_name
	);

	//	move into struct?
	let remote_socket_addr: SocketAddr = remote_addr.parse().unwrap();
	let stream = match TcpStream::connect_timeout(&remote_socket_addr, time::Duration::from_millis(1000)) {
		Ok(s) => s,
		Err(e) => {
			println!("Could not connect to '{}': {:?}", remote_addr, e);
			return;
		}
	};
	let cipher = get_local_to_outgoing_secure_stream_cipher(
		&stream,
		&local_key_pair,
		connection_guard.user.public_id.clone(),
	);
	let mut secure_stream: SecureStream = SecureStream::new(&stream, cipher);

	connection_guard.root_file = Some(request_file_list(
		&mut secure_stream,
		&connection_guard.user.display_name,
	));
	connection_guard.is_online = true;
	std::mem::drop(connection_guard);

	loop {
		let mut connection_guard = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());

		if connection_guard.is_deleted == true {
			println!("Outgoing Connection deleted. {}", connection_guard.user.display_name);
			std::mem::drop(connection_guard);
			break;
		}

		if connection_guard.should_reset_connection == true {
			println!("Outgoing Connection reset. {}", connection_guard.user.display_name);
			std::mem::drop(connection_guard);
			break;
		}

		if connection_guard.is_paused == true {
			std::mem::drop(connection_guard);
			thread::sleep(time::Duration::from_secs(3));
			continue;
		}

		let current_download_file = connection_guard.download_queue.get(0);
		match current_download_file {
			Some(file_path) => {
				let mut file_path = file_path.clone();
				println!(
					"PRE DOWNLOAD: {} - {} - {:?}",
					connection_guard.user.display_name, file_path, connection_guard.root_file
				);
				let remote_user_name = connection_guard.user.display_name.clone();
				file_path = file_path.replace("\\\\", "\\");
				let shared_file =
					get_file_by_path(&file_path, &connection_guard.root_file.clone().unwrap());
				
				if let None = shared_file {
					println!("Invalid download: {}", file_path);
					connection_guard.invalid_downloads.push(file_path);
					connection_guard.download_queue.pop_front();
					connection_guard.write_queue();
					connection_guard.active_download = None;
					std::mem::drop(connection_guard);
					continue;
				}

				let shared_file = shared_file.unwrap();
				connection_guard.stop_active_download = false;
				connection_guard.active_download_percent = 0;
				connection_guard.active_download_current_bytes = 0.0;
				connection_guard.active_download = Some(shared_file.clone());
				std::mem::drop(connection_guard);
				client_download_from_remote(
					&mut secure_stream,
					&shared_file,
					&remote_user_name,
					outgoing_connection,
				);
				let mut connection_guard = outgoing_connection
					.lock()
					.unwrap_or_else(|poisoned| poisoned.into_inner());
				if connection_guard.should_reset_connection == true || connection_guard.is_deleted == true {
					std::mem::drop(connection_guard);
					return;
				}

				if connection_guard.is_paused == true {
					std::mem::drop(connection_guard);
					continue;
				}

				connection_guard.active_download = None;
				
				if connection_guard.stop_active_download == false {
					if shared_file.is_directory {
						// FIXME copied from download_shared_file
						let destination_path = config::get_path_downloads_dir_user(&remote_user_name);
						let mut destination_path = destination_path.into_os_string().to_str().unwrap().to_string();
						destination_path.push_str("/");
						let current_path_obj = Path::new(&shared_file.path);
						let current_path_name = current_path_obj.file_name().unwrap().to_str().unwrap();
						destination_path.push_str(current_path_name);
						connection_guard.finished_downloads.push((shared_file.path, destination_path));
					}
				}
				
				// Cancelling all downloads clears the queue
				if connection_guard.download_queue.is_empty() == false {
					connection_guard.download_queue.pop_front();
				}
				connection_guard.write_queue();
				std::mem::drop(connection_guard);
			}
			None => {
				std::mem::drop(connection_guard);
				thread::sleep(time::Duration::from_millis(1000));
			}
		}
	}
}

pub fn get_local_to_outgoing_secure_stream_cipher(
	mut stream: &TcpStream,
	local_key_pair: &signature::Ed25519KeyPair,
	remote_public_id: String,
) -> Aes256Gcm {
	let local_diffie_secret = EphemeralSecret::new(OsRng);
	let local_diffie_public = PublicKey::from(&local_diffie_secret);
	let local_diffie_public_bytes: &[u8; 32] = local_diffie_public.as_bytes();
	let local_diffie_signature_public_bytes = local_key_pair.sign(local_diffie_public_bytes);
	let local_diffie_signed_public_bytes = local_diffie_signature_public_bytes.as_ref();
	// Send remote the local's diffie public key
	let mut buffer = [0; TOTAL_BUFFER_SIZE];
	let mut diffie_payload: Vec<u8> = Vec::with_capacity(32 + 64); // diffie public key + signature
	for byte in local_diffie_public_bytes {
		diffie_payload.push(byte.clone());
	}

	for byte in local_diffie_signed_public_bytes {
		diffie_payload.push(byte.clone());
	}

	set_buffer(&mut buffer, MSG_CLIENT_DIFFIE_PUBLIC, &diffie_payload);
	stream.write_all(&mut buffer).unwrap();
	stream.flush().unwrap();

	// Read remote diffie public
	stream.read_exact(&mut buffer).unwrap();
	let mut remote_diffie_public_bytes: [u8; 32] = [0; 32];
	remote_diffie_public_bytes.copy_from_slice(&buffer[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 32]);

	let mut remote_diffie_signed_public_bytes: [u8; 64] = [0; 64];
	remote_diffie_signed_public_bytes
		.copy_from_slice(&buffer[PAYLOAD_OFFSET + 32..PAYLOAD_OFFSET + 32 + 64]);


	let remote_public_key_bytes = crypto::get_bytes_from_base64_str(&remote_public_id);
	let remote_public_key =
		signature::UnparsedPublicKey::new(&signature::ED25519, remote_public_key_bytes);
	
	let client_connecting_addr = stream.peer_addr().unwrap();
	let client_connecting_ip = client_connecting_addr.ip().to_string();
	match remote_public_key
		.verify(
			&remote_diffie_public_bytes,
			&remote_diffie_signed_public_bytes,
		) {
			Err(_) => {
				panic!("ERROR: Remote signature failure. Verify Public ID is correct for {}", client_connecting_ip);
			}
			_ => {}
		}

	// Create encryption key
	let remote_diffie_public_key = PublicKey::from(remote_diffie_public_bytes);
	let local_shared_secret = local_diffie_secret.diffie_hellman(&remote_diffie_public_key);
	let encryption_key = local_shared_secret.as_bytes();
	let key = GenericArray::from_slice(&encryption_key[..]);
	// Create AES and stream
	let cipher = Aes256Gcm::new(key);
	return cipher;
}

fn client_download_from_remote(
	secure_stream: &mut SecureStream,
	download_file: &SharedFile,
	remote_user_name: &String,
	outgoing_connection: &Arc<Mutex<OutgoingConnection>>,
) {

	let root_download_dir = config::get_path_downloads_dir_user(remote_user_name);
	let mut root_download_dir = root_download_dir.into_os_string().to_str().unwrap().to_string();
	root_download_dir.push_str("/");
	println!("Download start: {:?}", download_file);
	download_shared_file(
		secure_stream,
		&download_file,
		&root_download_dir,
		false,
		outgoing_connection,
		&root_download_dir,
	);
}

fn download_shared_file(
	secure_stream: &mut SecureStream,
	shared_file: &SharedFile,
	download_dir: &String,
	continue_downloading_in_progress: bool,
	outgoing_connection: &Arc<Mutex<OutgoingConnection>>,
	root_download_dir: &String,
) {
	let current_path_obj = Path::new(&shared_file.path);
	let current_path_name = current_path_obj.file_name().unwrap().to_str().unwrap();

	if shared_file.is_directory {
		let mut new_download_dir = String::from(download_dir);
		new_download_dir.push_str(current_path_name);
		new_download_dir.push_str(&"/".to_string());
		for a_file in &shared_file.files {
			download_shared_file(
				secure_stream,
				a_file,
				&new_download_dir,
				continue_downloading_in_progress,
				outgoing_connection,
				root_download_dir,
			);
					
			let conn = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
			
			if conn.is_deleted == true {
				println!("Connection deleted mid DIR download. {}", conn.user.display_name);
				std::mem::drop(conn);
				return;
			}
			
			if conn.should_reset_connection == true {
				println!("Connection reset mid DIR download. {}", conn.user.display_name);
				std::mem::drop(conn);
				return;
			}

			if conn.stop_active_download == true {
				println!("Active download cancelled mid DIR download. {}", conn.user.display_name);
				std::mem::drop(conn);
				return;
			}

			std::mem::drop(conn);
		}
	} else {
		// Create directory for file download
		fs::create_dir_all(&download_dir).unwrap();
		let mut destination_path = download_dir.clone();
		destination_path.push_str(current_path_name);
		println!("Saving to: {}", destination_path);

		// Send selection to server
		println!("Sending selection to server");
		let selection_msg: u8;
		let selection_payload: Vec<u8>;
		let file_length: u64;
		if Path::new(&destination_path).exists() {
			file_length = metadata(&destination_path).unwrap().len();
		} else {
			file_length = 0;
		}
		let mut file_continue_payload = file_length.to_be_bytes().to_vec();
		file_continue_payload.extend_from_slice(&shared_file.path.as_bytes());
		selection_msg = MSG_FILE_SELECTION_CONTINUE;
		selection_payload = file_continue_payload;
		secure_stream.write(selection_msg, &selection_payload);

		// Check first response for error
		secure_stream.read();
		let mut server_msg: u8 = secure_stream.buffer[0];
		println!("Initial download response: {}", server_msg);

		if server_msg == MSG_FILE_INVALID_FILE {
			println!("!!!! ERROR: Invalid file selection");
			let mut conn = outgoing_connection
				.lock()
				.unwrap_or_else(|poisoned| poisoned.into_inner());
			conn.invalid_downloads.push(shared_file.path.clone());
			std::mem::drop(conn);
			return;
		}

		if server_msg == MSG_CANNOT_SELECT_DIRECTORY {
			panic!("!!!! ERROR: Cannot download directory");
		}

		if server_msg != MSG_FILE_CHUNK {
			panic!("!!!! ERROR: Expected MSG_FILE_CHUNK, got: {}", server_msg);
		}

		if server_msg == MSG_FILE_FINISHED {
			println!(
				"Download complete. Nothing left to download. {}",
				destination_path
			);
			return;
		}

		// Valid file, download it
		let mut current_downloaded_bytes: usize;
		let mut f: File;
		// TODO use .create() and remove else?
		if Path::new(&destination_path).exists() {
			f = OpenOptions::new()
				.write(true)
				.open(&destination_path)
				.unwrap();
			f.seek(SeekFrom::End(0)).unwrap();
			current_downloaded_bytes = metadata(&destination_path).unwrap().len() as usize;
		} else {
			f = File::create(&destination_path).unwrap();
			current_downloaded_bytes = 0;
		}
		// TODO Inefficient. Every download recalculates dir size
		let mut conn = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());

		// TODO send server message that mid download is cancelled
		if conn.is_deleted == true {
			println!("Connection deleted mid download. {}", conn.user.display_name);
			std::mem::drop(conn);
			return;
		}
		
		if conn.should_reset_connection == true {
			println!("Connection reset mid download. {}", conn.user.display_name);
			std::mem::drop(conn);
			return;
		}

		if conn.stop_active_download == true {
			println!("Active download cancelled. {}", conn.user.display_name);
			std::mem::drop(conn);
			return;
		}
		
		if conn.active_download.as_ref().unwrap().is_directory {
			let _path_obj = Path::new(&conn.active_download.as_ref().unwrap().path);
			let _path_name = _path_obj.file_name().unwrap().to_str().unwrap();

			let mut dldir = String::from(root_download_dir);
			dldir.push_str("/");
			dldir.push_str(_path_name);
			conn.active_download_current_bytes = get_size_of_directory(&dldir) as f64;
		} else {
			conn.active_download_current_bytes = current_downloaded_bytes as f64;
		}
		std::mem::drop(conn);

		loop {
			let actual_payload_size = get_payload_size_from_buffer(&secure_stream.buffer);
			current_downloaded_bytes += actual_payload_size;
			f.write(&secure_stream.buffer[PAYLOAD_OFFSET..actual_payload_size + PAYLOAD_OFFSET])
				.unwrap();
			secure_stream.read();
			server_msg = secure_stream.buffer[0];

			let mut conn = outgoing_connection
				.lock()
				.unwrap_or_else(|poisoned| poisoned.into_inner());

			if conn.is_deleted == true {
				println!("Connection deleted in download. {}", conn.user.display_name);
				std::mem::drop(conn);
				return;
			}

			if conn.should_reset_connection == true {
				println!("Connection reset in download. {}", conn.user.display_name);
				std::mem::drop(conn);
				return;
			}

			if conn.stop_active_download == true {
				println!("Active download cancelled in download. {}", conn.user.display_name);
				std::mem::drop(conn);
				return;
			}
			
			conn.active_download_current_bytes += actual_payload_size as f64;
			conn.active_download_percent = ((conn.active_download_current_bytes as f64
				/ conn.active_download.clone().unwrap().file_size as f64)
				* (100 as f64)) as usize;
			std::mem::drop(conn);

			if server_msg == MSG_FILE_FINISHED {
				break;
			}
			if server_msg != MSG_FILE_CHUNK {
				panic!("Expected MSG_FILE_CHUNK, got {}", server_msg);
			}
		}

		let mut conn = outgoing_connection
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		//conn.active_download_percent = 0;
		//conn.active_download = None;
		conn.finished_downloads.push((shared_file.path.clone(), destination_path.clone()));
		std::mem::drop(conn);
		println!("100%\nDownload finished: {}", destination_path);
	}
}