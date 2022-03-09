use ring::{
	rand,
	signature::{self, KeyPair},
};
use std::{error::Error, vec::Vec};
extern crate base64;

// was ring::error::Unspecified
pub fn generate_id_pair() -> Result<(Vec<u8>, Vec<u8>), Box<dyn Error>> {
	// Generate a key pair in PKCS#8 (v2) format.
	let rng = rand::SystemRandom::new();
	let pkcs8_bytes = match signature::Ed25519KeyPair::generate_pkcs8(&rng) {
        Ok(pkcs8_bytes) => pkcs8_bytes,
        Err(e) => {
            Err(format!("Failed to generate pkcs8 bytes. {}", e.to_string()))?
        },
    };

	let key_pair = match signature::Ed25519KeyPair::from_pkcs8(pkcs8_bytes.as_ref()) {
        Ok(key_pair) => key_pair,
        Err(e) => {
            Err(format!("Key Pair rejected. {}", e.to_string()))?
        },
    };

	let public_key_bytes = key_pair.public_key().as_ref();

	return Ok((pkcs8_bytes.as_ref().to_vec(), public_key_bytes.to_vec()));
}

pub fn get_bytes_from_base64_str(string: &String) -> Result<Vec<u8>, Box<dyn Error>> {
	let decoded = base64::decode(string)?;
	return Ok(decoded);
}

pub fn get_base64_str_from_bytes(bytes: Vec<u8>) -> String {
	let encoded = base64::encode(bytes);
	return encoded;
}