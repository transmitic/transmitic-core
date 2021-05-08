use core::time;
use std::{collections::{HashMap, VecDeque}, fs::OpenOptions, io::SeekFrom, net::{Shutdown, TcpListener, TcpStream}, panic, path::Path, sync::{Arc, Mutex, MutexGuard}, thread};
use std::{net::SocketAddr};

use crate::{config::{self, Config, TrustedUser}, core_consts::{MAX_DATA_SIZE, MSG_CANNOT_SELECT_DIRECTORY, MSG_CLIENT_DIFFIE_PUBLIC, MSG_FILE_CHUNK, MSG_FILE_FINISHED, MSG_FILE_INVALID_FILE, MSG_FILE_LIST, MSG_FILE_LIST_FINAL, MSG_FILE_LIST_PIECE, MSG_FILE_SELECTION_CONTINUE, MSG_SERVER_DIFFIE_PUBLIC, PAYLOAD_OFFSET, TOTAL_BUFFER_SIZE}, crypto, secure_stream::SecureStream, shared_file::{FileToDownload, SharedFile, get_everything_file}, utils::{LocalKeyData, get_file_by_path, get_payload_size_from_buffer, get_size_of_directory, request_file_list, set_buffer}};

use aes_gcm::aead::{generic_array::GenericArray, Aead, NewAead};
use aes_gcm::Aes256Gcm;
use rand_core::OsRng;
use ring::{
	rand,
	signature::{self, KeyPair},
};
use x25519_dalek::EphemeralSecret;

use std::fs;
use std::fs::metadata;
use std::fs::File;
use std::io::prelude::*;
use x25519_dalek::PublicKey;


pub struct IncomingConnectionManager {
	pub config: Arc<Mutex<Config>>,
	pub incoming_connections: Arc<Mutex<HashMap<String, Arc<Mutex<IncomingConnection>>>>>,

}

impl IncomingConnectionManager {
	pub fn new(config: Arc<Mutex<Config>>, ) -> IncomingConnectionManager {

		let config_guard = config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		let mut incoming_connections: HashMap<String, Arc<Mutex<IncomingConnection>>> = HashMap::new();

		for user in config_guard.trusted_users_public_ids.iter() {
			let incoming_connection = IncomingConnection {
				user: user.clone(),
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
			incoming_connections.insert(user.display_name.clone(), incomig_mutex);
		}

		std::mem::drop(config_guard);
		let arc_incoming = Arc::new(Mutex::new(incoming_connections));

		IncomingConnectionManager {
			config: config,
			incoming_connections: arc_incoming,
		}
	}

	// TODO share reset functions with self functions
	pub fn reset_all_connections_for_all_users(&mut self) {
		let incoming_connections_guard = self.incoming_connections
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		
		for (display_name, incoming_connection) in incoming_connections_guard.iter() {
			let mut connection_guard = incoming_connection
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
			connection_guard = reset_all_connections(connection_guard);
			std::mem::drop(connection_guard);

		}

		std::mem::drop(incoming_connections_guard);
	}

	pub fn reset_all_connections_for_specific_users(&mut self, shared_with_display_names: &Vec<String>) {
		let incoming_connections_guard = self.incoming_connections
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		
		for (display_name, incoming_connection) in incoming_connections_guard.iter() {
			if shared_with_display_names.contains(display_name) == true {
				let mut connection_guard = incoming_connection
				.lock()
				.unwrap_or_else(|poisoned| poisoned.into_inner());
				connection_guard = reset_all_connections(connection_guard);
				std::mem::drop(connection_guard);
			}
		}

		std::mem::drop(incoming_connections_guard);
	}

	pub fn reset_all_connections(&mut self, display_name: &String) {
		let incoming_connections_guard = self.incoming_connections
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	
		let incoming_connection = incoming_connections_guard
		.get(display_name)
		.unwrap();
	
		let mut connection_guard = incoming_connection
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		connection_guard = reset_all_connections(connection_guard);
		std::mem::drop(connection_guard);
		std::mem::drop(incoming_connections_guard);
	}

	pub fn remove_user(&mut self, display_name: &String) {
		let mut incoming_connections_guard = self.incoming_connections
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		let incoming_connection = incoming_connections_guard
		.get(display_name)
		.unwrap();

		let mut connection_guard = incoming_connection
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		
		connection_guard.is_disabled = true;
		std::mem::drop(connection_guard);

		incoming_connections_guard.remove(display_name);

		std::mem::drop(incoming_connections_guard);
	}

	// TODO share this function with other self functions that disable
	pub fn disable_user(&mut self, display_name: &String) {
		let incoming_connections_guard = self.incoming_connections
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		let incoming_connection = incoming_connections_guard
		.get(display_name)
		.unwrap();

		let mut connection_guard = incoming_connection
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		connection_guard.is_disabled = true;
		std::mem::drop(connection_guard);
		std::mem::drop(incoming_connections_guard);
	}

	pub fn enable_user(&mut self, display_name: &String) {
		let incoming_connections_guard = self.incoming_connections
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		let incoming_connection = incoming_connections_guard
		.get(display_name)
		.unwrap();

		let mut connection_guard = incoming_connection
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		connection_guard.is_disabled = false;
		std::mem::drop(connection_guard);
		std::mem::drop(incoming_connections_guard);
	}

}

fn reset_all_connections(mut connection_guard: MutexGuard<IncomingConnection>) -> MutexGuard<IncomingConnection> {
	println!("Start reset all connections");
	for s in connection_guard.single_connections.iter_mut() {
		s.lock().unwrap().should_reset = true;
	}
	connection_guard.single_connections.clear();
	println!("End reset all connections");
	return connection_guard;
}

pub struct SingleConnection {
	pub should_reset: bool,
}

pub struct IncomingConnection {
	pub user: TrustedUser,
	pub active_download: Option<SharedFile>,
	pub active_download_percent: usize,
	pub active_download_current_bytes: f64,
	pub is_downloading: bool,
	pub finished_downloads: Vec<String>,
	pub is_disabled: bool,
	pub single_connections: Vec<Arc<Mutex<SingleConnection>>>,
}

fn client_handle_incoming(
	incoming_connections: Arc<Mutex<HashMap<String, Arc<Mutex<IncomingConnection>>>>>,
	mut stream: &TcpStream,
	config: Arc<Mutex<Config>>,
	local_key_pair_bytes: Vec<u8>,
) {
	let client_connecting_addr = stream.peer_addr().unwrap();
	let client_connecting_ip = client_connecting_addr.ip().to_string();

	// Find client config
	let mut client_config: Option<&TrustedUser> = None;
	let config_guard = config			
	.lock()
	.unwrap_or_else(|poisoned| poisoned.into_inner());
	for client in config_guard.trusted_users_public_ids.iter() {
		if client.ip_address == client_connecting_ip {
			client_config = Some(client);
		}
	}

	let client_config = match client_config {
		Some(found) => Some(found),
		None => {
			println!("!!!! WARNING: Rejected unknown IP: {}", client_connecting_ip);
			stream
				.shutdown(Shutdown::Both)
				.expect("shutdown call failed");
			println!("\tConnection has been shutdown");
			return;
		}
	};

	let remote_config = client_config.unwrap();

	let current_incoming_ip = client_connecting_ip.clone();
	let current_incoming_public_id = remote_config.public_id.clone();

	// Check if disabled
	let incoming_connections_guard = incoming_connections
	.lock()
	.unwrap_or_else(|poisoned| poisoned.into_inner());

	let incoming_connection = incoming_connections_guard
	.get(&remote_config.display_name)
	.unwrap();

	let mut connection_guard = incoming_connection
	.lock()
	.unwrap_or_else(|poisoned| poisoned.into_inner());
	let is_disabled = connection_guard.is_disabled;
	let single_connection = SingleConnection {
		should_reset: false,
	};
	let single_connection_arc = Arc::new(Mutex::new(single_connection));
	let single_connection_arc_clone = Arc::clone(&single_connection_arc);
	connection_guard.single_connections.push(single_connection_arc_clone);
	std::mem::drop(connection_guard);
	std::mem::drop(incoming_connections_guard);
	if is_disabled == true {
		std::mem::drop(config_guard);
		println!("!!!! WARNING: Disabled user connecting. Rejected.: {}", client_connecting_ip);
		stream
			.shutdown(Shutdown::Both)
			.expect("shutdown call failed");
		println!("\tConnection has been shutdown");
		return;
	}

	println!("Connected: {}", remote_config.display_name);
	let local_key_pair =
		signature::Ed25519KeyPair::from_pkcs8(local_key_pair_bytes.as_ref()).unwrap();
	let local_diffie_secret = EphemeralSecret::new(OsRng);
	let local_diffie_public = PublicKey::from(&local_diffie_secret);
	let local_diffie_public_bytes: &[u8; 32] = local_diffie_public.as_bytes();
	let local_diffie_signature_public_bytes = local_key_pair.sign(local_diffie_public_bytes);
	let local_diffie_signed_public_bytes = local_diffie_signature_public_bytes.as_ref();
	// Wait for remote diffie public key
	let mut buffer = [0; TOTAL_BUFFER_SIZE];
	stream.read_exact(&mut buffer).unwrap();
	let mut remote_diffie_public_bytes: [u8; 32] = [0; 32];
	remote_diffie_public_bytes.copy_from_slice(&buffer[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 32]);

	let mut remote_diffie_signed_public_bytes: [u8; 64] = [0; 64];
	remote_diffie_signed_public_bytes
		.copy_from_slice(&buffer[PAYLOAD_OFFSET + 32..PAYLOAD_OFFSET + 32 + 64]);

	let remote_public_key_bytes = crypto::get_bytes_from_base64_str(&remote_config.public_id);
	let remote_public_key =
		signature::UnparsedPublicKey::new(&signature::ED25519, remote_public_key_bytes);
	match remote_public_key.verify(
		&remote_diffie_public_bytes,
		&remote_diffie_signed_public_bytes,
	) {
		Err(_) => {
			panic!(
				"ERROR: Incoming connection failed signature. Public key isn't valid. {} - {}",
				remote_config.display_name, client_connecting_ip
			);
		}
		_ => {}
	}

	// Send remote the local's diffie public key
	let mut diffie_payload: Vec<u8> = Vec::with_capacity(32 + 64); // diffie public key + signature
	for byte in local_diffie_public_bytes {
		diffie_payload.push(byte.clone());
	}

	for byte in local_diffie_signed_public_bytes {
		diffie_payload.push(byte.clone());
	}
	set_buffer(&mut buffer, MSG_SERVER_DIFFIE_PUBLIC, &diffie_payload);
	stream.write_all(&mut buffer).unwrap();
	stream.flush().unwrap();

	// Create encryption key
	let remote_diffie_public_key = PublicKey::from(remote_diffie_public_bytes);
	let local_shared_secret = local_diffie_secret.diffie_hellman(&remote_diffie_public_key);
	let encryption_key = local_shared_secret.as_bytes();
	let key = GenericArray::from_slice(&encryption_key[..]);
	// Create AES and stream
	let cipher = Aes256Gcm::new(key);
	let mut secure_stream: SecureStream = SecureStream::new(stream, cipher);

	// Get list of files shared with user
	let everything_file = get_everything_file(&config_guard, &remote_config.display_name);
	let everything_file_json: String = serde_json::to_string(&everything_file).unwrap();
	let everything_file_json_bytes = everything_file_json.as_bytes().to_vec();

	// IncomingConnection
	let incoming_connections_guard = incoming_connections
	.lock()
	.unwrap_or_else(|poisoned| poisoned.into_inner());

	let incoming_connection = incoming_connections_guard
	.get(&remote_config.display_name)
	.unwrap();

	let incoming_connection_clone = Arc::clone(&incoming_connection);

	// TODO should this be dropped earlier? When is this needed?
	std::mem::drop(incoming_connections_guard);
	std::mem::drop(config_guard);


	loop {
		let mut connection_guard = incoming_connection_clone
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		// Check if IP or Public ID has changed
		let has_public_id_changed = connection_guard.user.public_id != current_incoming_public_id;
		let has_ip_changed = connection_guard.user.ip_address != current_incoming_ip;
		if has_ip_changed {
			println!("Incoming conn. IP changed for {}. {} -> {}", connection_guard.user.display_name, current_incoming_ip, connection_guard.user.ip_address);
		}
		if has_public_id_changed {
			println!("Incoming conn. Public ID changed for {}. {} -> {}", connection_guard.user.display_name, current_incoming_public_id, connection_guard.user.public_id);
		}
		let should_reset_connection = single_connection_arc.lock().unwrap().should_reset;
		let should_stop = connection_guard.is_disabled || has_public_id_changed || has_ip_changed || should_reset_connection;
		if should_stop == true {
			connection_guard.is_downloading = false;
			println!("Incoming conn stopped. {}", connection_guard.user.display_name);
		}
		if should_reset_connection {
			println!("Incoming conn reset. {}", connection_guard.user.display_name);
		}
		std::mem::drop(connection_guard);

		if should_stop == true {
			break;
		}

		client_handle_incoming_loop(
			&incoming_connection_clone,
			&single_connection_arc,
			&mut secure_stream,
			&everything_file_json_bytes,
			&everything_file,
		);
	}
}

fn client_handle_incoming_loop(
	incoming_connection: &Arc<Mutex<IncomingConnection>>,
	single_connection_arc: &Arc<Mutex<SingleConnection>>,
	secure_stream: &mut SecureStream,
	everything_file_json_bytes: &Vec<u8>,
	everything_file: &SharedFile,
) {
	println!("Wait for client");

	if single_connection_arc.lock().unwrap().should_reset {
		return;
	}

	secure_stream.read();

	if single_connection_arc.lock().unwrap().should_reset {
		return;
	}

	let client_msg = secure_stream.buffer[0];

	if client_msg == MSG_FILE_LIST {
		println!("Client requests file list");
		if everything_file_json_bytes.len() <= MAX_DATA_SIZE {
			// TODO is this branch not needed?
			secure_stream.write(MSG_FILE_LIST_FINAL, &everything_file_json_bytes);
		} else {
			let mut remaining_bytes = everything_file_json_bytes.len();
			let mut sent_bytes = 0;
			loop {
				if remaining_bytes <= MAX_DATA_SIZE {
					let send_vec = Vec::from(&everything_file_json_bytes[sent_bytes..remaining_bytes+sent_bytes]);
					secure_stream.write(MSG_FILE_LIST_FINAL, &send_vec);
					break;
				} else {
					let send_vec = Vec::from(&everything_file_json_bytes[sent_bytes..MAX_DATA_SIZE+sent_bytes]);
					secure_stream.write(MSG_FILE_LIST_PIECE, &send_vec);
				}
				
				sent_bytes += MAX_DATA_SIZE;
				remaining_bytes -= MAX_DATA_SIZE;
			}
		}
		println!("File list sent");
	} else if client_msg == MSG_FILE_SELECTION_CONTINUE {
		println!("Client sent file selection for downloading");

		// Get client's file selection and optional seek point if continuing download
		let actual_payload_size = get_payload_size_from_buffer(&secure_stream.buffer);
		let mut payload_bytes: Vec<u8> = Vec::with_capacity(actual_payload_size);
		for mut i in 0..actual_payload_size {
			i += PAYLOAD_OFFSET;
			payload_bytes.push(secure_stream.buffer[i]);
		}

		let file_seek_point: u64;
		let client_file_choice: &str;
		let mut seek_bytes: [u8; 8] = [0; 8];
		seek_bytes.copy_from_slice(&payload_bytes[0..8]);
		file_seek_point = u64::from_be_bytes(seek_bytes);
		client_file_choice = std::str::from_utf8(&payload_bytes[8..]).unwrap();

		println!("    File seek point: {}", file_seek_point);
		println!("    Client chose file {}", client_file_choice);

		// Determine if client's choice is valid
		let client_shared_file = match get_file_by_path(client_file_choice, &everything_file) {
			Some(file) =>  file,
			None => {
				println!("    ! Invalid file choice");				
				secure_stream.write(MSG_FILE_INVALID_FILE, &Vec::with_capacity(1));
				if single_connection_arc.lock().unwrap().should_reset {
					return;
				}
				return;
			}
		};

		// Client cannot select a directory. Client should not allow this to happen.
		if client_shared_file.is_directory {
			println!("    ! Selected directory. Not allowed.");
			secure_stream.write(MSG_CANNOT_SELECT_DIRECTORY, &Vec::with_capacity(1));
			return;
		}

		let mut connection_guard = incoming_connection
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		connection_guard.is_downloading = true;
		connection_guard.active_download = Some(client_shared_file.clone());
		std::mem::drop(connection_guard);

		// Send file to client
		let mut f = OpenOptions::new()
			.read(true)
			.open(client_file_choice)
			.unwrap();
		f.seek(SeekFrom::Start(file_seek_point)).unwrap();

		println!("Start sending file");

		let mut read_response = 1; // TODO combine with loop?
		let mut read_buffer = [0; MAX_DATA_SIZE];
		let mut current_sent_bytes: usize = file_seek_point as usize;
		let mut download_percent: f64;
		let file_size_f64: f64 = client_shared_file.file_size as f64;
		while read_response != 0 {
			read_response = f.read(&mut read_buffer).unwrap();
			secure_stream.write(MSG_FILE_CHUNK, &read_buffer[0..read_response].to_vec());
			current_sent_bytes += read_response;
			download_percent = ((current_sent_bytes as f64) / file_size_f64) * (100 as f64);
			let mut connection_guard = incoming_connection
				.lock()
				.unwrap_or_else(|poisoned| poisoned.into_inner());
			connection_guard.active_download_percent = download_percent as usize;
			if single_connection_arc.lock().unwrap().should_reset == true || connection_guard.is_disabled == true {
				std::mem::drop(connection_guard);
				return;
			}
			std::mem::drop(connection_guard);
		}

		// Finished sending file
		println!("Send message finished");
		secure_stream.write(MSG_FILE_FINISHED, &Vec::with_capacity(1));
		println!("File transfer complete");
		let mut connection_guard = incoming_connection
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner());
		connection_guard.is_downloading = false;
		connection_guard
			.finished_downloads
			.push(client_shared_file.path);
		std::mem::drop(connection_guard);
	} else {
		panic!("Invalid client selection {}", client_msg);
	}
}

pub fn client_wait_for_incoming(
	incoming_connections: Arc<Mutex<HashMap<String, Arc<Mutex<IncomingConnection>>>>>,
	config: Arc<Mutex<Config>>,
	local_key_data: Arc<Mutex<LocalKeyData>>,
	sharing_mode_arc: Arc<Mutex<String>>,
) {

	println!("Client wait for incoming");

	loop {



		let sharing_guard = sharing_mode_arc	
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());

		let sharing_mode = sharing_guard.clone();

		std::mem::drop(sharing_guard);

		

		

		if sharing_mode == "Off" {
			thread::sleep(time::Duration::from_secs(1));
			continue;
		}

		let config_guard = config			
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
		let mut ip_address = String::new();

		


		if sharing_mode == "Local Network" {
			ip_address.push_str("127.0.0.1");
		} else if sharing_mode == "Internet" {
			ip_address.push_str("0.0.0.0");
		} else {
			std::mem::drop(config_guard);
			panic!(
				"Server flag invalid. 'Local Network' or 'Internet' is valid. Yours -> {}",
				sharing_mode
			);
		}
		ip_address.push_str(":");
		ip_address.push_str(&config_guard.server_port);
		println!(
			"\nWaiting for clients on: {}",
			sharing_mode
		);

		std::mem::drop(config_guard);

		println!("Server waiting for incoming connections...");
		println!("{}", ip_address);
		let listener = TcpListener::bind(ip_address).unwrap();
		listener.set_nonblocking(true).expect("Cannot set non-blocking");


		for stream in listener.incoming() {

			match stream {
				Ok(s) => {
					let sharing_guard = sharing_mode_arc	
					.lock()
					.unwrap_or_else(|poisoned| poisoned.into_inner());
			
					let new_sharing_mode = sharing_guard.clone();
			
					std::mem::drop(sharing_guard);

					if new_sharing_mode == "Off" || new_sharing_mode != sharing_mode {
						s.shutdown(Shutdown::Both).expect("shutdown call failed");
						break;
					}

					s.set_nonblocking(false).unwrap();

					let config_clone = Arc::clone(&config);
					let local_key_data_guard = local_key_data
					.lock()
					.unwrap_or_else(|poisoned| poisoned.into_inner());
					let local_key_pair_bytes_clone = local_key_data_guard.local_key_pair_bytes.clone();
					std::mem::drop(local_key_data_guard);
			
					let incoming_clone = Arc::clone(&incoming_connections);
					thread::spawn(move || {
						let client_connecting_ip = s.peer_addr().unwrap().ip().to_string();
						println!("Client connecting: {}", client_connecting_ip);
						let _ = panic::catch_unwind(|| {
							client_handle_incoming(
								incoming_clone,
								&s,
								config_clone,
								local_key_pair_bytes_clone,
							);
						});
						s
							.shutdown(Shutdown::Both)
							.expect("shutdown call failed");
						println!("Connection ended: {}", client_connecting_ip);
					});
				}
				Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
					let sharing_guard = sharing_mode_arc	
					.lock()
					.unwrap_or_else(|poisoned| poisoned.into_inner());
			
					let new_sharing_mode = sharing_guard.clone();
			
					std::mem::drop(sharing_guard);

					if new_sharing_mode == "Off" || new_sharing_mode != sharing_mode {
						break;
					}
					thread::sleep(time::Duration::from_secs(1));

				}
				Err(e) => {
					println!("ERROR: Failed initial client connection");
					println!("{:?}", e);
					continue;
				}
			}
			
		}
		std::mem::drop(listener);
	}

}