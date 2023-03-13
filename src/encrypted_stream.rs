use std::convert::TryInto;
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::{thread, time};

use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::aead::AeadMut;
use aes_gcm::{Aes256Gcm, KeyInit};

use crate::core_consts::{
    CRC_MESSAGES, CRC_SIZE, MSG_TYPE_SIZE, PAYLOAD_OFFSET, PAYLOAD_SIZE_LEN, TOTAL_BUFFER_SIZE,
    TOTAL_CRYPTO_BUFFER_SIZE,
};
use crate::crypto::{NONCE_INIT, NONCE_MAX};

// TODO Read and write only payload size, not entire buffer
//  and encrypt only needed
//  Zero out buffer?

pub struct EncryptedStream {
    stream: TcpStream,
    cipher: Aes256Gcm,
    nonce: u128,
    crypto_buffer: Vec<u8>,
    pub buffer: Vec<u8>,
}

impl EncryptedStream {
    pub fn new(stream: TcpStream, encryption_key: [u8; 32]) -> EncryptedStream {
        let key = GenericArray::from_slice(&encryption_key[..]);
        // Create AES and stream
        let cipher = Aes256Gcm::new(key);
        let nonce: u128 = NONCE_INIT;
        let crypto_buffer: Vec<u8> = vec![0; TOTAL_CRYPTO_BUFFER_SIZE];
        let buffer: Vec<u8> = vec![0; TOTAL_BUFFER_SIZE];

        EncryptedStream {
            stream,
            cipher,
            nonce,
            crypto_buffer,
            buffer,
        }
    }

    fn increment_nonce(&mut self) -> Result<(), Box<dyn Error>> {
        if self.nonce >= NONCE_MAX {
            return Err("Nonce maxed. Reconnect.".into());
        }
        self.nonce += 1;
        Ok(())
    }

    fn get_nonce_bytes(&self) -> Result<[u8; 12], Box<dyn Error>> {
        let nonce_bytes = self.nonce.to_be_bytes();
        let nonce_sub: [u8; 12] = nonce_bytes[4..].try_into()?;
        Ok(nonce_sub)
    }

    pub fn read(&mut self) -> Result<(), Box<dyn Error>> {
        self._read_stream()?;
        let nonce_bytes = self.get_nonce_bytes()?;
        let new_nonce = GenericArray::from_slice(&nonce_bytes);

        let plaintext = match self.cipher.decrypt(new_nonce, self.crypto_buffer.as_ref()) {
            Ok(plaintext) => plaintext,
            Err(e) => return Err(format!("Encrypted Stream read failed to decrypt. {}", e).into()),
        };

        let _ = &mut self.buffer.copy_from_slice(&plaintext[..TOTAL_BUFFER_SIZE]);
        self.increment_nonce()?;
        Ok(())
    }

    fn _read_stream(&mut self) -> Result<(), std::io::Error> {
        self.stream.read_exact(&mut self.crypto_buffer)
    }

    pub fn write(&mut self, msg: u16, payload: &[u8]) -> Result<(), Box<dyn Error>> {
        self.set_buffer(msg, payload);
        let nonce_bytes = self.get_nonce_bytes()?;
        let new_nonce = GenericArray::from_slice(&nonce_bytes);

        let cipher_text = match self.cipher.encrypt(new_nonce, self.buffer.as_ref()) {
            Ok(cipher_text) => cipher_text,
            Err(e) => return Err(format!("Encrypted Stream write failed to encrypt. {}", e).into()),
        };

        self._write_stream(&cipher_text[..])?;
        self.increment_nonce()?;
        Ok(())
    }

    fn _write_stream(&mut self, buffer: &[u8]) -> Result<(), Box<dyn Error>> {
        self.stream.write_all(buffer)?;
        Ok(())
    }

    pub fn get_message(&self) -> Result<u16, Box<dyn Error>> {
        let message_array = self.buffer[0..2].try_into()?;
        Ok(u16::from_be_bytes(message_array))
    }

    pub fn get_payload(&self) -> Result<&[u8], Box<dyn Error>> {
        let payload_size_bytes: u32 = u32::from_be_bytes(
            self.buffer[MSG_TYPE_SIZE..PAYLOAD_SIZE_LEN + MSG_TYPE_SIZE].try_into()?,
        );
        let payload_size = payload_size_bytes as usize;
        let payload = &self.buffer[PAYLOAD_OFFSET..PAYLOAD_OFFSET + payload_size];

        // CRC
        let message = self.get_message()?;
        if self.is_crc_payload(message) {
            let expected_checksum_bytes = &self.buffer
                [PAYLOAD_OFFSET + payload_size..PAYLOAD_OFFSET + payload_size + CRC_SIZE];
            let expected_checksum = u32::from_be_bytes(expected_checksum_bytes[..].try_into()?);
            let actual_checksum = crc32fast::hash(payload);

            if expected_checksum != actual_checksum {
                thread::sleep(time::Duration::from_secs(30));
                Err(format!(
                    "Checksum mismatch. Actual: '{:?}' - Expected '{:?}'",
                    actual_checksum, expected_checksum
                ))?;
            }
        }

        Ok(payload)
    }

    fn set_buffer(&mut self, message: u16, payload: &[u8]) {
        // TODO check payload size and err if too big?
        let message_bytes = &message.to_be_bytes();
        self.buffer[..MSG_TYPE_SIZE].copy_from_slice(message_bytes);

        // Get size of payload
        let payload_size: usize = payload.len();
        let payload_size_u32: u32 = payload_size as u32;
        let payload_size_bytes = payload_size_u32.to_be_bytes();

        // Set size of payload
        self.buffer[MSG_TYPE_SIZE..PAYLOAD_SIZE_LEN + MSG_TYPE_SIZE]
            .copy_from_slice(&payload_size_bytes[0..PAYLOAD_SIZE_LEN]);

        // Set payload
        self.buffer[PAYLOAD_OFFSET..PAYLOAD_OFFSET + payload_size].copy_from_slice(payload);

        // CRC
        if self.is_crc_payload(message) {
            let checksum = crc32fast::hash(payload);
            let checksum_bytes = checksum.to_be_bytes();
            println!("{:?}", checksum);
            self.buffer[PAYLOAD_OFFSET + payload_size..PAYLOAD_OFFSET + payload_size + CRC_SIZE]
                .copy_from_slice(&checksum_bytes);
        }
    }

    fn is_crc_payload(&self, message: u16) -> bool {
        CRC_MESSAGES.contains(&message)
    }
}
