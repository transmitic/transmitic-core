use std::{
	fs::{self, metadata}, 
	process
};

use ring::signature;

use crate::{
	core_consts::{MSG_FILE_LIST, MSG_FILE_LIST_FINAL, MSG_FILE_LIST_PIECE, PAYLOAD_OFFSET, PAYLOAD_SIZE_LEN}, 
	secure_stream::SecureStream, 
	shared_file::{SharedFile, remove_invalid_files}
};

// TODO should be core?
pub fn get_blocked_file_name_chars() -> String {
    return String::from("{};*?'\"<>|");
}

// TODO should be core?
pub fn get_blocked_display_name_chars() -> String {
    let mut chars = get_blocked_file_name_chars();
    chars.push_str("/\\[]()");
    return chars;
}

// TODO should core do this?
pub fn exit_error(msg: String) -> ! {
    println!("\n!!!! ERROR: {}", msg);
    println!("Transmitic has stopped");
    process::exit(1);
}

// TODO should be core?
pub fn get_file_size_string(mut bytes: u64) -> String {
    let gig: u64 = 1_000_000_000;
    let meg: u64 = 1_000_000;
    let byte: u64 = 1000;

    let divisor: u64;
    let unit: String;

    if bytes == 0 {
        bytes = 1;
        divisor = 1;
        unit = "b".to_string();
    } else if bytes >= gig {
        divisor = gig;
        unit = "GB".to_string();
    } else if bytes >= meg {
        divisor = meg;
        unit = "MB".to_string();
    } else if bytes >= byte {
        divisor = byte;
        unit = "KB".to_string();
    } else {
        divisor = byte;
        unit = "b".to_string();
    }

    let size = bytes as f64 / divisor as f64;
    let mut size_string = String::from(format!("{:.2}", size));
    size_string.push_str(" ");
    size_string.push_str(&unit);
    return size_string;
}

// TODO move
pub fn set_buffer(buffer: &mut [u8], msg_type: u8, payload: &Vec<u8>) {
    buffer[0] = msg_type;

    // Get size of payload
    let payload_size: usize = payload.len();
    let payload_size_u32: u32 = payload_size as u32;
    let payload_size_bytes = payload_size_u32.to_be_bytes();

    // Set size of payload
    buffer[1..PAYLOAD_SIZE_LEN + 1].copy_from_slice(&payload_size_bytes[0..PAYLOAD_SIZE_LEN]);

    // Set payload
    buffer[PAYLOAD_OFFSET..PAYLOAD_OFFSET + payload_size].copy_from_slice(&payload);
}

// TODO move
pub struct LocalKeyData {
	pub local_key_pair: signature::Ed25519KeyPair,
	pub local_key_pair_bytes: Vec<u8>,
}

pub fn get_payload_size_from_buffer(buffer: &[u8]) -> usize {
	let mut payload_size_bytes = [0; PAYLOAD_SIZE_LEN];
	payload_size_bytes[..].copy_from_slice(&buffer[1..PAYLOAD_SIZE_LEN + 1]);
	let payload_size: usize = u32::from_be_bytes(payload_size_bytes) as usize;
	payload_size
}

pub fn get_size_of_directory(path: &str) -> usize {
	let mut size: usize = 0;
	for entry in fs::read_dir(path).unwrap() {
		let entry = entry.unwrap();
		let path = entry.path();
		let path_str = path.to_str().unwrap();
		if path.is_dir() {
			size += get_size_of_directory(path_str);
		} else {
			size += metadata(path_str).unwrap().len() as usize;
		}
	}

	return size;
}

pub fn request_file_list(secure_stream: &mut SecureStream, client_display_name: &String) -> SharedFile {
	// Request file list
	let mut payload_bytes: Vec<u8> = Vec::new();

	secure_stream.write(MSG_FILE_LIST, &Vec::with_capacity(1));

	// Receive file list
	loop {
		secure_stream.read();
		let server_msg: u8 = secure_stream.buffer[0];
		if server_msg != MSG_FILE_LIST_PIECE && server_msg != MSG_FILE_LIST_FINAL {
			exit_error(format!("Request file list got unexpected MSG: '{:?}'", server_msg));
		}
		let actual_payload_size = get_payload_size_from_buffer(&secure_stream.buffer);
		
		payload_bytes.extend_from_slice(&secure_stream.buffer[PAYLOAD_OFFSET..PAYLOAD_OFFSET+actual_payload_size]);
		
		if server_msg == MSG_FILE_LIST_FINAL {
			break;
		}
	}

	// Create FilesJson struct
	let files_str = std::str::from_utf8(&payload_bytes).unwrap();
	let mut all_files: SharedFile = serde_json::from_str(&files_str).unwrap();
	//println!("{:?}", all_files);

	// Keep valid file names
	remove_invalid_files(&mut all_files, &client_display_name);

	return all_files;
}

pub fn get_file_by_path(file_choice: &str, shared_file: &SharedFile) -> Option<SharedFile> {
	if shared_file.path == file_choice {
		return Some(shared_file.clone());
	}

	for a_file in &shared_file.files {
		if let Some(found) =  get_file_by_path(file_choice, &a_file) {
			return Some(found.clone());
		}
	}

	return None;
}