use alloc::vec::Vec;

mod index;

struct PackObjectLocation {
    pack_file_name: Vec<u8>,
    offset: u64,
}

