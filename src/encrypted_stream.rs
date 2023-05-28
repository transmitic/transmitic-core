use std::convert::TryInto;
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::{thread, time};

use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::aead::AeadMut;
use aes_gcm::{Aes256Gcm, KeyInit};

use crate::config::SharedUser;
use crate::core_consts::{
    CRC_MESSAGES, CRC_SIZE, CRYPTO_EXTENSION_SIZE, MSG_TYPE_SIZE, PAYLOAD_OFFSET, PAYLOAD_SIZE_LEN,
    TOTAL_BUFFER_SIZE, TOTAL_CRYPTO_BUFFER_SIZE,
};
use crate::crypto::{NONCE_INIT, NONCE_MAX};

pub struct EncryptedStream {
    stream: TcpStream,
    cipher: Aes256Gcm,
    nonce: u128,
    crypto_buffer: Vec<u8>,
    pub buffer: Vec<u8>,
    pub shared_user: SharedUser,
    pub private_id_bytes: Vec<u8>,
}

impl EncryptedStream {
    pub fn new(
        stream: TcpStream,
        encryption_key: [u8; 32],
        shared_user: SharedUser,
        private_id_bytes: Vec<u8>,
    ) -> EncryptedStream {
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
            shared_user,
            private_id_bytes,
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
        // Read message stream
        let read_size = PAYLOAD_OFFSET + CRYPTO_EXTENSION_SIZE;
        self._read_stream(read_size)?;

        let nonce_bytes = self.get_nonce_bytes()?;
        let new_nonce = GenericArray::from_slice(&nonce_bytes);

        let plaintext_msg = match self
            .cipher
            .decrypt(new_nonce, self.crypto_buffer[..read_size].as_ref())
        {
            Ok(plaintext) => plaintext,
            Err(e) => {
                return Err(
                    format!("Encrypted Stream read failed to decrypt message. {}", e).into(),
                )
            }
        };
        let _ = &mut self.buffer[..plaintext_msg.len()].copy_from_slice(&plaintext_msg[..]);
        self.increment_nonce()?;

        let payload_size_bytes: u32 = u32::from_be_bytes(
            self.buffer[MSG_TYPE_SIZE..PAYLOAD_OFFSET].try_into()?,
        );
        let payload_size = payload_size_bytes as usize;
        let msg = self.get_message()?;

        // No payload, therefore nothing else to decrypt
        if payload_size == 0 {
            return Ok(());
        }

        // Read payload stream
        let mut read_size: usize = payload_size + CRYPTO_EXTENSION_SIZE;
        if self.is_crc_payload(msg) {
            read_size += CRC_SIZE;
        }
        self._read_stream(read_size)?;

        let nonce_bytes = self.get_nonce_bytes()?;
        let new_nonce = GenericArray::from_slice(&nonce_bytes);

        let plaintext_payload = match self
            .cipher
            .decrypt(new_nonce, self.crypto_buffer[..read_size].as_ref())
        {
            Ok(plaintext) => plaintext,
            Err(e) => {
                return Err(
                    format!("Encrypted Stream read failed to decrypt payload. {}", e).into(),
                )
            }
        };
        let _ = &mut self.buffer
            [plaintext_msg.len()..plaintext_msg.len() + plaintext_payload.len()]
            .copy_from_slice(&plaintext_payload[..]);
        self.increment_nonce()?;

        Ok(())
    }

    fn _read_stream(&mut self, read_size: usize) -> Result<(), std::io::Error> {
        self.stream.read_exact(&mut self.crypto_buffer[..read_size])
    }

    pub fn write(&mut self, msg: u16, payload: &[u8]) -> Result<(), Box<dyn Error>> {
        self.write_message(msg, payload)?;
        if !payload.is_empty() {
            self.write_payload(msg, payload)?;
        }

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
            self.buffer[MSG_TYPE_SIZE..PAYLOAD_OFFSET].try_into()?,
        );
        let payload_size = payload_size_bytes as usize;
        let payload = &self.buffer[PAYLOAD_OFFSET..PAYLOAD_OFFSET + payload_size];

        // CRC
        let message = self.get_message()?;
        if self.is_crc_payload(message) && payload_size > 0 {
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

    fn write_message(&mut self, msg: u16, payload: &[u8]) -> Result<(), Box<dyn Error>> {
        // TODO check payload size and err if too big?
        let message_bytes = &msg.to_be_bytes();
        self.buffer[..MSG_TYPE_SIZE].copy_from_slice(message_bytes);

        // Get size of payload
        let payload_size: usize = payload.len();
        let payload_size_u32: u32 = payload_size as u32;
        let payload_size_bytes = payload_size_u32.to_be_bytes();

        // Set size of payload
        self.buffer[MSG_TYPE_SIZE..PAYLOAD_OFFSET]
            .copy_from_slice(&payload_size_bytes[0..PAYLOAD_SIZE_LEN]);

        let nonce_bytes = self.get_nonce_bytes()?;
        let new_nonce = GenericArray::from_slice(&nonce_bytes);

        let cipher_text = match self.cipher.encrypt(
            new_nonce,
            self.buffer[..PAYLOAD_OFFSET].as_ref(),
        ) {
            Ok(cipher_text) => cipher_text,
            Err(e) => {
                return Err(
                    format!("Encrypted Stream write failed to encrypt message. {}", e).into(),
                )
            }
        };

        self._write_stream(&cipher_text[..])?;
        self.increment_nonce()?;

        Ok(())
    }

    fn write_payload(&mut self, msg: u16, payload: &[u8]) -> Result<(), Box<dyn Error>> {
        let mut used_buffer_len = 0;
        // Get size of payload
        let payload_size: usize = payload.len();
        used_buffer_len += payload_size;

        // Set payload
        self.buffer[..payload_size].copy_from_slice(payload);

        // CRC
        if self.is_crc_payload(msg) {
            let checksum = crc32fast::hash(payload);
            let checksum_bytes = checksum.to_be_bytes();
            self.buffer[payload_size..payload_size + CRC_SIZE].copy_from_slice(&checksum_bytes);
            used_buffer_len += CRC_SIZE;
        }

        let nonce_bytes = self.get_nonce_bytes()?;
        let new_nonce = GenericArray::from_slice(&nonce_bytes);

        let cipher_text = match self
            .cipher
            .encrypt(new_nonce, self.buffer[..used_buffer_len].as_ref())
        {
            Ok(cipher_text) => cipher_text,
            Err(e) => {
                return Err(
                    format!("Encrypted Stream write failed to encrypt payload. {}", e).into(),
                )
            }
        };

        self._write_stream(&cipher_text[..])?;
        self.increment_nonce()?;

        Ok(())
    }

    fn is_crc_payload(&self, message: u16) -> bool {
        CRC_MESSAGES.contains(&message)
    }
}
