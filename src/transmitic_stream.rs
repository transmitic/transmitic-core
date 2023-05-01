use std::{
    error::Error,
    io::{Read, Write},
    net::TcpStream,
};

use rand_core::OsRng;
use ring::signature;
use x25519_dalek::{EphemeralSecret, PublicKey};

use crate::{
    config::SharedUser,
    core_consts::{TRAN_API_MAJOR, TRAN_API_MINOR, TRAN_MAGIC_NUMBER},
    crypto,
    encrypted_stream::EncryptedStream,
    outgoing_downloader::ERR_REMOTE_ID_NOT_FOUND,
};

const BUFFER_SIZE: usize = 32 + 64;

// TODO drop struct and just use functions instead?
pub struct TransmiticStream {
    stream: TcpStream,
    connecting_ip: String,
    shared_users: Vec<SharedUser>,
    private_key_pair: signature::Ed25519KeyPair,
    private_id_bytes: Vec<u8>,
    matched_shared_user: Option<SharedUser>,
}

impl TransmiticStream {
    pub fn new(
        stream: TcpStream,
        connecting_ip: String,
        shared_users: Vec<SharedUser>,
        private_id_bytes: Vec<u8>,
    ) -> TransmiticStream {
        // TODO function
        let private_key_pair =
            signature::Ed25519KeyPair::from_pkcs8(private_id_bytes.as_ref()).unwrap();

        stream.set_nonblocking(false).unwrap();

        TransmiticStream {
            stream,
            connecting_ip,
            shared_users,
            private_key_pair,
            private_id_bytes,
            matched_shared_user: None,
        }
    }

    pub fn connect(&mut self) -> Result<EncryptedStream, Box<dyn Error>> {
        self.send_transmitic_header()?;
        self.receive_transmitic_header()?;
        let local_diffie_secret = self.send_diffie_helman_key()?;
        let remote_diffie_key = self.receive_diffie_helman_key()?;
        let encryption_key = self.get_encryption_key(local_diffie_secret, remote_diffie_key);
        let encrypted_stream = self.get_encrypted_stream(encryption_key)?;
        Ok(encrypted_stream)
    }

    pub fn wait_for_incoming(&mut self) -> Result<EncryptedStream, Box<dyn Error>> {
        self.receive_transmitic_header()?;
        self.send_transmitic_header()?;
        let remote_diffie_key = self.receive_diffie_helman_key()?;
        let local_diffie_secret = self.send_diffie_helman_key()?;
        let encryption_key = self.get_encryption_key(local_diffie_secret, remote_diffie_key);
        let encrypted_stream = self.get_encrypted_stream(encryption_key)?;
        Ok(encrypted_stream)
    }

    fn get_encrypted_stream(
        &self,
        encryption_key: [u8; 32],
    ) -> Result<EncryptedStream, Box<dyn Error>> {
        // TODO can i shutdown the original stream?
        let cloned_stream = self.stream.try_clone()?;

        let shared_user = match &self.matched_shared_user {
            Some(shared_user) => shared_user.to_owned(),
            None => {
                eprintln!("SharedUser not matched for encryption stream");
                std::process::exit(1);
            }
        };
        let encrypted_stream = EncryptedStream::new(
            cloned_stream,
            encryption_key,
            shared_user,
            self.private_id_bytes.clone(),
        );
        Ok(encrypted_stream)
    }

    fn send_transmitic_header(&mut self) -> Result<(), Box<dyn Error>> {
        let mut buffer: [u8; 7] = [0; TRAN_MAGIC_NUMBER.len() + 1 + 2];
        buffer[0..4].copy_from_slice(&TRAN_MAGIC_NUMBER);
        buffer[4] = TRAN_API_MAJOR;
        buffer[5..7].copy_from_slice(&TRAN_API_MINOR.to_be_bytes());
        self.stream.write_all(&buffer)?;
        Ok(())
    }

    fn receive_transmitic_header(&mut self) -> Result<(), Box<dyn Error>> {
        let mut buffer: [u8; 7] = [0; TRAN_MAGIC_NUMBER.len() + 1 + 2];
        // TODO set read timeout
        self.stream.read_exact(&mut buffer)?;

        if buffer[0..4] != TRAN_MAGIC_NUMBER {
            return Err(format!(
                "{} TRAN MAGIC NUMBER mismatch. {:?}",
                self.connecting_ip,
                &buffer[0..4]
            )
            .into());
        }

        if buffer[4] != TRAN_API_MAJOR {
            return Err(format!(
                "{} TRAN API MAJOR mismatch. {}",
                self.connecting_ip, &buffer[4]
            )
            .into());
        }

        Ok(())
    }

    fn send_diffie_helman_key(&mut self) -> Result<EphemeralSecret, Box<dyn Error>> {
        let local_diffie_secret = EphemeralSecret::new(OsRng);
        let local_diffie_public = PublicKey::from(&local_diffie_secret);
        let local_diffie_public_bytes: &[u8; 32] = local_diffie_public.as_bytes();
        let local_diffie_signature_public_bytes =
            self.private_key_pair.sign(local_diffie_public_bytes);
        let local_diffie_signed_public_bytes = local_diffie_signature_public_bytes.as_ref();
        // diffie public key + diffie public key signed
        let mut buffer = [0; BUFFER_SIZE];
        buffer[0..32].copy_from_slice(&local_diffie_public_bytes[0..32]);
        buffer[32..BUFFER_SIZE].copy_from_slice(&local_diffie_signed_public_bytes[0..64]);

        self.stream.write_all(&buffer)?;

        Ok(local_diffie_secret)
    }

    fn receive_diffie_helman_key(&mut self) -> Result<x25519_dalek::PublicKey, Box<dyn Error>> {
        let mut buffer: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];
        // TODO set read timeout
        self.stream.read_exact(&mut buffer)?;
        // Get diffie bytes from buffer
        let mut remote_diffie_public_bytes: [u8; 32] = [0; 32];
        remote_diffie_public_bytes.copy_from_slice(&buffer[0..32]);

        // Get signed diffie bytes from buffer
        let mut remote_diffie_signed_public_bytes: [u8; 64] = [0; 64];
        remote_diffie_signed_public_bytes[..].copy_from_slice(&buffer[32..BUFFER_SIZE]);

        // Try every shared user
        for shared_user in &self.shared_users {
            // Load PublicID from shared_user
            let remote_public_id_bytes = crypto::get_bytes_from_base64_str(&shared_user.public_id)?;
            let remote_public_key =
                signature::UnparsedPublicKey::new(&signature::ED25519, remote_public_id_bytes);

            // Verify diffie bytes were signed with the PublicID we have for this user
            if remote_public_key
                .verify(
                    &remote_diffie_public_bytes,
                    &remote_diffie_signed_public_bytes,
                )
                .is_ok()
            {
                // Found SharedUser
                self.matched_shared_user = Some(shared_user.to_owned());
                break;
            }
        }

        // No SharedUser could be found. This is an unknown user.
        if self.matched_shared_user.is_none() {
            return Err(ERR_REMOTE_ID_NOT_FOUND.to_string().into());
        }

        // Create the diffie public key now that it's been verified
        let remote_diffie_public_key = PublicKey::from(remote_diffie_public_bytes);
        Ok(remote_diffie_public_key)
    }

    fn get_encryption_key(
        &self,
        local_diffie_secret: EphemeralSecret,
        remote_diffie_public_key: PublicKey,
    ) -> [u8; 32] {
        let local_shared_secret = local_diffie_secret.diffie_hellman(&remote_diffie_public_key);
        let encryption_key = local_shared_secret.as_bytes();
        *encryption_key
    }
}
