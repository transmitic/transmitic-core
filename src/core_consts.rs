// TODO update ! after beta to N
// TODO upgrade TRAN API MAJOR TO u16
pub const TRAN_MAGIC_NUMBER: [u8; 4] = [b'T', b'R', b'A', b'!'];
pub const TRAN_API_MAJOR: u8 = 2; // TODO upgrade TRAN API MAJOR TO u16
pub const TRAN_API_MINOR: u16 = 0;

// TODO REORG all
// TODO ue repr enum below instead?
pub const MSG_TYPE_SIZE: usize = 2;
pub const PAYLOAD_SIZE_LEN: usize = 4;

pub const MSG_FILE_LIST: u16 = 1;
pub const MSG_FILE_LIST_PIECE: u16 = 2;
pub const MSG_FILE_LIST_FINAL: u16 = 3;

pub const MSG_FILE_CHUNK: u16 = 4;
pub const MSG_FILE_FINISHED: u16 = 5;

pub const MSG_FILE_INVALID_FILE: u16 = 6;
pub const MSG_CANNOT_SELECT_DIRECTORY: u16 = 7;
pub const MSG_FILE_SELECTION_CONTINUE: u16 = 8; // TODO Only use CONTINUE?

pub const MAX_DATA_SIZE: usize = 100_000;
pub const TOTAL_BUFFER_SIZE: usize = MSG_TYPE_SIZE + PAYLOAD_SIZE_LEN + MAX_DATA_SIZE;
pub const TOTAL_CRYPTO_BUFFER_SIZE: usize = TOTAL_BUFFER_SIZE + 16;
pub const PAYLOAD_OFFSET: usize = MSG_TYPE_SIZE + PAYLOAD_SIZE_LEN;

// #[repr(u16)]
// enum Mes {
//     msg_file_list = 100000000,
//     msg_file = 2,

// }
