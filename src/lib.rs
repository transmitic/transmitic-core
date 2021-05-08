#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}

pub fn hello() {
    println!("HOLC");
}

pub mod crypto;
pub mod config;
pub mod utils;
pub mod shared_file;
pub mod secure_stream;
pub mod core_consts;
pub mod outgoing;
pub mod incoming;
pub mod transmitic_core;