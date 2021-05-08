use crate::core_consts::{TOTAL_BUFFER_SIZE, TOTAL_CRYPTO_BUFFER_SIZE};
use crate::utils::set_buffer;
use aes_gcm::aead::{generic_array::GenericArray, Aead};
use aes_gcm::Aes256Gcm;
use std::io::prelude::*;
use std::net::TcpStream;

pub struct SecureStream<'a> {
    pub tcp_stream: &'a TcpStream,
    nonce: [u8; 12],
    cipher: Aes256Gcm,
    crypto_buffer: [u8; TOTAL_CRYPTO_BUFFER_SIZE],
    pub buffer: [u8; TOTAL_BUFFER_SIZE],
}

impl<'a> SecureStream<'a> {
    pub fn new(tcp_stream: &'a TcpStream, cipher: Aes256Gcm) -> SecureStream<'a> {
        SecureStream {
            tcp_stream,
            nonce: [0; 12],
            cipher,
            crypto_buffer: [0; TOTAL_CRYPTO_BUFFER_SIZE],
            buffer: [0; TOTAL_BUFFER_SIZE],
        }
    }

    fn increment_nonce(&mut self) {
        let mut flip: bool;
        for i in 0..self.nonce.len() {
            let byte = self.nonce[i];
            // TODO once this maxes out, we have to reset connection
            if byte >= 255 {
                if i == self.nonce.len() - 1 {
                    panic!("ERROR: Nonce maxed. Reconnect.");
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
    }

    pub fn read(&mut self) {
        self._read_stream();
        let new_nonce = GenericArray::from_slice(&self.nonce[..]);
        let plaintext = self
            .cipher
            .decrypt(new_nonce, self.crypto_buffer.as_ref())
            .unwrap();
        &mut self.buffer.copy_from_slice(&plaintext[..TOTAL_BUFFER_SIZE]);
        self.increment_nonce();
    }

    fn _read_stream(&mut self) {
        self.tcp_stream.read_exact(&mut self.crypto_buffer).unwrap();
    }

    pub fn write(&mut self, msg: u8, payload: &Vec<u8>) {
        set_buffer(&mut self.buffer, msg, payload);
        let new_nonce = GenericArray::from_slice(&self.nonce[..]);
        let cipher_text = self
            .cipher
            .encrypt(new_nonce, self.buffer.as_ref())
            .unwrap();

        self._write_stream(&cipher_text[..]);
        self.increment_nonce();
    }

    fn _write_stream(&mut self, buffer: &[u8]) {
        self.tcp_stream.write_all(buffer).unwrap();
        self.tcp_stream.flush().unwrap();
    }
}
