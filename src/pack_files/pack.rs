use crate::{directory::Directory, error::GResult, pack_files::PackObjectLocation, repo::Repo};
use alloc::vec::Vec;

enum PackObjectType {
    Commit,
    Blob,
    Tree,
    Tag,
    OffsetDelta,
    RefDelta,
}

async fn read_pack_object<D: Directory>(
    _repo: &Repo<D>,
    _location: &PackObjectLocation,
) -> GResult<(PackObjectType, Vec<u8>)> {
    todo!()
}

#[cfg(test)]
mod tests {

    #[test]
    fn read_non_deltified_object() {
        todo!()
    }
}
