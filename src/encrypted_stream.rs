use std::convert::TryInto;
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpStream;

use aes_gcm::aead::{generic_array::GenericArray, NewAead};
use aes_gcm::Aes256Gcm;

use crate::core_consts::{TOTAL_BUFFER_SIZE, PAYLOAD_SIZE_LEN, PAYLOAD_OFFSET, TOTAL_CRYPTO_BUFFER_SIZE, MSG_TYPE_SIZE};

// TODO Read and write only payload size, not entire buffer
//  and encrypt only needed
//  Zero out buffer

pub struct EncryptedStream {
    stream: TcpStream,
    cipher: Aes256Gcm,
    nonce: [u8; 12],
    crypto_buffer: Vec<u8>,
    pub buffer: Vec<u8>,
}

impl EncryptedStream {

    pub fn new(stream: TcpStream, encryption_key: [u8; 32]) -> EncryptedStream {
        let key = GenericArray::from_slice(&encryption_key[..]);
        // Create AES and stream
        let cipher = Aes256Gcm::new(key);
        let nonce = [0; 12];
        let crypto_buffer: Vec<u8> = vec![0; TOTAL_CRYPTO_BUFFER_SIZE];
        let buffer: Vec<u8> = vec![0; TOTAL_BUFFER_SIZE];

        return EncryptedStream {
            stream, 
            cipher,
            nonce,
            crypto_buffer,
            buffer,
        }
    }

    fn increment_nonce(&mut self) -> Result<(), Box<dyn Error>> {
        let mut flip: bool;
        for i in 0..self.nonce.len() {
            let byte = self.nonce[i];
            if byte >= 255 {
                if i == self.nonce.len() - 1 {
                    return Err("ERROR: Nonce maxed. Reconnect.")?;
                }
                self.nonce[i] = 0;
                flip = true;
            } else {
                self.nonce[i] += 1;
                flip = false;
            }

            if flip == false {
                break;
            }
        }

        return Ok(());
    }

    pub fn read(&mut self) -> Result<(), Box<dyn Error>> {
        self._read_stream()?;
        let new_nonce = GenericArray::from_slice(&self.nonce[..]);
        let plaintext = match aes_gcm::aead::Aead::decrypt(&self
            .cipher, new_nonce, self.crypto_buffer.as_ref()) {
                Ok(plaintext) => plaintext,
                Err(e) => Err(format!("Encrypted Stream read failed to decrypt. {}", e.to_string()))?,
            };
        let _ = &mut self.buffer.copy_from_slice(&plaintext[..TOTAL_BUFFER_SIZE]);
        self.increment_nonce()?;
        return Ok(());
    }

    fn _read_stream(&mut self) -> Result<(), std::io::Error> {
        let result = self.stream.read_exact(&mut self.crypto_buffer);
        return result;
    }

    pub fn write(&mut self, msg: u16, payload: &Vec<u8>) -> Result<(), Box<dyn Error>> {
        self.set_buffer(msg, payload);
        let new_nonce = GenericArray::from_slice(&self.nonce[..]);
        let cipher_text = match aes_gcm::aead::Aead::encrypt(&self
            .cipher, new_nonce, self.buffer.as_ref()) {
                Ok(cipher_text) => cipher_text,
                Err(e) => Err(format!("Encrypted Stream write failed to encrypt. {}", e.to_string()))?,
            };

        self._write_stream(&cipher_text[..])?;
        self.increment_nonce()?;
        return Ok(());
    }

    fn _write_stream(&mut self, buffer: &[u8]) -> Result<(), Box<dyn Error>>{
        self.stream.write_all(buffer)?;
        return Ok(());
    }

    pub fn get_message(&self) -> Result<u16, Box<dyn Error>> {
        let message_array = self.buffer[0..2].try_into()?;
        return Ok(u16::from_be_bytes(message_array));
    }

    pub fn get_payload(&self) -> Result<&[u8], Box<dyn Error>> {
        let payload_size_bytes: u32 = u32::from_be_bytes(self.buffer[MSG_TYPE_SIZE..PAYLOAD_SIZE_LEN + MSG_TYPE_SIZE].try_into()?);
        let payload_size = payload_size_bytes as usize;
        return Ok(&self.buffer[PAYLOAD_OFFSET..PAYLOAD_OFFSET + payload_size]);
    }

    fn set_buffer(&mut self, message: u16, payload: &Vec<u8>) {
        // TODO check payload size and err if too big?
        let message_bytes = &message.to_be_bytes();
        self.buffer[..MSG_TYPE_SIZE].copy_from_slice(message_bytes);

        // Get size of payload
        let payload_size: usize = payload.len();
        let payload_size_u32: u32 = payload_size as u32;
        let payload_size_bytes = payload_size_u32.to_be_bytes();

        // Set size of payload
        self.buffer[MSG_TYPE_SIZE..PAYLOAD_SIZE_LEN + MSG_TYPE_SIZE].copy_from_slice(&payload_size_bytes[0..PAYLOAD_SIZE_LEN]);

        // Set payload
        self.buffer[PAYLOAD_OFFSET..PAYLOAD_OFFSET + payload_size].copy_from_slice(&payload);
    }

}
