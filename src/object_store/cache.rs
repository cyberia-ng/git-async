use crate::{
    error::GResult,
    file_system::{DirEntry, Directory},
    object_store::{
        index::{FanoutTable, ShortOffsetTable},
        lookup::PackName,
        pack::validate_packfile_version,
    },
    traits::AllGenerics,
};
use alloc::vec::Vec;

pub(crate) struct IndexCache<G: AllGenerics> {
    pub pack_dir: G::Directory,
    pub indexes: Vec<(PackName, FanoutTable, ShortOffsetTable)>,
}

impl<G: AllGenerics> IndexCache<G> {
    pub async fn new(git_dir: &G::Directory) -> GResult<Self> {
        let pack_dir = git_dir
            .open_subdir(b"objects")
            .await?
            .open_subdir(b"pack")
            .await?;
        let pack_ids: Vec<PackName> = pack_dir
            .list_dir()
            .await?
            .into_iter()
            .filter_map(|dirent| -> Option<PackName> {
                use DirEntry::*;
                let File(name) = dirent else { None? };
                PackName::new(name)
            })
            .collect();
        let mut fanouts = Vec::with_capacity(pack_ids.len());
        for pack_id in pack_ids {
            let mut file = pack_dir.open_file(&pack_id.index_filename).await?;
            let fanout = FanoutTable::load(&mut file).await?;
            let offset_table = ShortOffsetTable::load(&mut file, fanout.total_objects()).await?;
            let mut pack_file = pack_dir.open_file(&pack_id.pack_filename).await?;
            validate_packfile_version(&mut pack_file).await?;
            fanouts.push((pack_id, fanout, offset_table));
        }
        Ok(Self {
            pack_dir,
            indexes: fanouts,
        })
    }
}
