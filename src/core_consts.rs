// TODO update ! after beta to N
pub const TRAN_MAGIC_NUMBER: [u8; 4] = ['T' as u8, 'R' as u8, 'A' as u8, '!' as u8];
pub const TRAN_API_MAJOR: u8 = 1;
pub const TRAN_API_MINOR: u16 = 0;


// CONNECTION ESTABLISH
pub const CONN_ESTABLISH_REQUEST: u16 = 0;
pub const CONN_ESTABLISH_ACCEPT: u16 = 1;
pub const CONN_ESTABLISH_REJECT: u16 = 2;

//TODO REORG all

pub const MSG_TYPE_SIZE: usize = 1;
pub const PAYLOAD_SIZE_LEN: usize = 4;

pub const MSG_FILE_LIST: u8 = 1;
pub const MSG_FILE_LIST_PIECE: u8 = 2;
pub const MSG_FILE_LIST_FINAL: u8 = 3;

pub const MSG_FILE_CHUNK: u8 = 4;
pub const MSG_FILE_FINISHED: u8 = 5;

pub const MSG_FILE_INVALID_FILE: u8 = 6;
pub const MSG_CANNOT_SELECT_DIRECTORY: u8 = 7;
pub const MSG_FILE_SELECTION_CONTINUE: u8 = 8; // TODO Only use CONTINUE?

pub const MSG_CLIENT_DIFFIE_PUBLIC: u8 = 9;
pub const MSG_SERVER_DIFFIE_PUBLIC: u8 = 10;

pub const MAX_DATA_SIZE: usize = 100_000;
pub const TOTAL_BUFFER_SIZE: usize = MSG_TYPE_SIZE + PAYLOAD_SIZE_LEN + MAX_DATA_SIZE;
pub const TOTAL_CRYPTO_BUFFER_SIZE: usize = TOTAL_BUFFER_SIZE + 16;
pub const PAYLOAD_OFFSET: usize = MSG_TYPE_SIZE + PAYLOAD_SIZE_LEN;