use std::{net::{TcpStream}, io::{Write, Read}, error::Error};

use rand_core::OsRng;
use ring::signature;
use x25519_dalek::{EphemeralSecret, PublicKey};

use crate::{core_consts::{TRAN_MAGIC_NUMBER, TRAN_API_MAJOR, TRAN_API_MINOR}, config::SharedUser, crypto, encrypted_stream::EncryptedStream};


// TODO drop struct and just use functions instead?
pub struct TransmiticStream {
    stream: TcpStream,
    shared_user: SharedUser,
    private_key_pair:  signature::Ed25519KeyPair,
    private_id_bytes: Vec<u8>,
}

impl TransmiticStream {

    pub fn new(stream: TcpStream, shared_user: SharedUser, private_id_bytes:  Vec<u8>) -> TransmiticStream {

        // TODO function
        let private_key_pair =
        signature::Ed25519KeyPair::from_pkcs8(private_id_bytes.as_ref()).unwrap();

        stream.set_nonblocking(false).unwrap();

        return TransmiticStream {
            stream: stream,
            shared_user,
            private_key_pair,
            private_id_bytes,
        }

    }

    pub fn connect(&mut self) -> Result<EncryptedStream, Box<dyn Error>> {
        self.send_transmitic_header()?;
        self.receive_transmitic_header()?;
        let local_diffie_secret = self.send_diffie_helman_key()?;
        let remote_diffie_key = self.receive_diffie_helman_key()?;
        let encryption_key = self.get_encryption_key(local_diffie_secret, remote_diffie_key);
        let encrypted_stream = self.get_encrypted_stream(encryption_key)?;
        return Ok(encrypted_stream);
    }

    pub fn wait_for_incoming(&mut self) -> Result<EncryptedStream, Box<dyn Error>> {
        self.receive_transmitic_header()?;
        self.send_transmitic_header()?;
        let remote_diffie_key = self.receive_diffie_helman_key()?;
        let local_diffie_secret = self.send_diffie_helman_key()?;
        let encryption_key = self.get_encryption_key(local_diffie_secret, remote_diffie_key);
        let encrypted_stream = self.get_encrypted_stream(encryption_key)?;
        return Ok(encrypted_stream);
    }

    fn get_encrypted_stream(&self, encryption_key: [u8; 32]) -> Result<EncryptedStream, Box<dyn Error>> {
        // TODO can i shutdown the original stream?
        let cloned_stream = self.stream.try_clone()?;
        let encrypted_stream = EncryptedStream::new(cloned_stream, encryption_key);
        return Ok(encrypted_stream);
    }

    fn send_transmitic_header(&mut self) -> Result<(), Box<dyn Error>> {
        let mut buffer: [u8; 7] = [0; 4 + 1 + 2];
        buffer[0..4].copy_from_slice(&TRAN_MAGIC_NUMBER);
        buffer[4] = TRAN_API_MAJOR;
        buffer[5..7].copy_from_slice(&TRAN_API_MINOR.to_be_bytes());
        self.stream.write_all(&buffer)?;
        return Ok(());
    }

    fn receive_transmitic_header(&mut self) -> Result<(), Box<dyn Error>> {
        let mut buffer: [u8; 7] = [0; 4 + 1 + 2];
        // TODO set read timeout
        self.stream.read_exact(&mut buffer)?;

        if buffer[0..4] != TRAN_MAGIC_NUMBER {
            return Err(format!("{} TRAN MAGIC NUMBER mismatch. {:?}", self.shared_user.nickname, &buffer[0..4]))?;
        }

        if buffer[4] != TRAN_API_MAJOR {
            return Err(format!("{} TRAN API MAJOR mismatch. {}", self.shared_user.nickname, &buffer[4]))?;
        }

        return Ok(());
    }

    fn send_diffie_helman_key(&mut self) -> Result<EphemeralSecret, Box<dyn Error>> {
        let local_diffie_secret = EphemeralSecret::new(OsRng);
        let local_diffie_public = PublicKey::from(&local_diffie_secret);
        let local_diffie_public_bytes: &[u8; 32] = local_diffie_public.as_bytes();
        let local_diffie_signature_public_bytes = self.private_key_pair.sign(local_diffie_public_bytes);
        let local_diffie_signed_public_bytes = local_diffie_signature_public_bytes.as_ref();
        // diffie public key + diffie public key signed
        const buffer_size: usize = 32 + 64;
        let mut buffer = [0; buffer_size];
        buffer[0..32].copy_from_slice(&local_diffie_public_bytes[0..32]);
        buffer[32..buffer_size].copy_from_slice(&local_diffie_signed_public_bytes[0..64]);

        self.stream.write_all(&buffer)?;

        return Ok(local_diffie_secret);
    }

    fn receive_diffie_helman_key(&mut self) -> Result<x25519_dalek::PublicKey, Box<dyn Error>> {
        const buffer_size: usize = 32 + 64;
        let mut buffer: [u8; buffer_size] = [0; buffer_size];
        // TODO set read timeout
        self.stream.read_exact(&mut buffer)?;
        // Get diffie bytes from buffer
        let mut remote_diffie_public_bytes: [u8; 32] = [0; 32];
        remote_diffie_public_bytes.copy_from_slice(&buffer[0..32]);

        // Get signed diffie bytes from buffer
        let mut remote_diffie_signed_public_bytes: [u8; 64] = [0; 64];
        remote_diffie_signed_public_bytes[..].copy_from_slice(&buffer[32..buffer_size]);

        // Load PublicID from shared_user
        let remote_public_id_bytes = crypto::get_bytes_from_base64_str(&self.shared_user.public_id)?;
        let remote_public_key =
		signature::UnparsedPublicKey::new(&signature::ED25519, remote_public_id_bytes);

        // Verify diffie bytes were signed with the PublicID we have for this user
        match remote_public_key
		.verify(
			&remote_diffie_public_bytes,
			&remote_diffie_signed_public_bytes,
		) {
            Ok(_) => {},
            Err(e) => {
                return Err(format!("Remote ID does not match. {}", e.to_string()))?;
            },
            }
        
        // Create the diffie public key now that it's been verified
        let remote_diffie_public_key = PublicKey::from(remote_diffie_public_bytes);
        return Ok(remote_diffie_public_key);

    }

    fn get_encryption_key(&self, local_diffie_secret: EphemeralSecret, remote_diffie_public_key: PublicKey) -> [u8; 32] {
        let local_shared_secret = local_diffie_secret.diffie_hellman(&remote_diffie_public_key);
        let encryption_key = local_shared_secret.as_bytes();
        return *encryption_key;
    }

}