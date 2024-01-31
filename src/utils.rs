use std::{
    error::Error,
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    str::FromStr,
    sync::mpsc::Sender,
    time,
};

use crate::{app_aggregator::AppAggMessage, config::SharedUser, shared_file::SharedFile};

pub fn get_file_by_path(file_choice: &str, shared_file: &SharedFile) -> Option<SharedFile> {
    if shared_file.path == file_choice {
        return Some(shared_file.clone());
    }

    for a_file in &shared_file.files {
        if let Some(found) = get_file_by_path(file_choice, a_file) {
            return Some(found);
        }
    }

    None
}

pub fn resolve_address(
    host: &String,
    port: &String,
    shared_user: &SharedUser,
    app_sender: &Sender<AppAggMessage>,
) -> Result<Vec<SocketAddr>, String> {
    // If already IPv4/6, just return it
    if let Ok(addr) = SocketAddr::from_str(&format!("{}:{}", host, port)) {
        if addr.is_ipv4() || addr.is_ipv6() {
            app_sender
                .send(AppAggMessage::LogDebug(format!(
                    "Resolved: Already an IP '{}'",
                    addr.ip()
                )))
                .ok();
            return Ok(vec![addr]);
        }
    }

    // Could be IPv6 that's missing []
    if let Ok(addr) = SocketAddr::from_str(&format!("[{}]:{}", host, port)) {
        if addr.is_ipv6() {
            app_sender
                .send(AppAggMessage::LogDebug(format!(
                    "Resolved: Already an IP, modified '{}'",
                    addr.ip()
                )))
                .ok();
            return Ok(vec![addr]);
        }
    }

    let address = format!("{}:{}", host, port);
    let resolved_address_vec: Vec<SocketAddr> = match address.to_socket_addrs() {
        Ok(a) => a.collect(),
        Err(e) => Err(format!(
            "Failed to resolve host for '{}'. {}",
            shared_user.nickname, e
        ))?,
    };

    let all_addresses: Vec<String> = resolved_address_vec.iter().map(|f| f.to_string()).collect();

    app_sender
        .send(AppAggMessage::LogDebug(format!(
            "Resolved: IP '{}' -> '{:?}'",
            address, all_addresses
        )))
        .ok();
    Ok(resolved_address_vec)
}

pub fn tcp_connect(socket_addrs: &Vec<SocketAddr>) -> Result<TcpStream, Box<dyn Error>> {
    for socket in socket_addrs {
        match TcpStream::connect_timeout(socket, time::Duration::from_secs(2)) {
            Ok(s) => return Ok(s),
            Err(_) => continue,
        }
    }

    Err("Timed out trying to connect to all socket addrs")?
}
