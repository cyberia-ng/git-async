use crate::{
    error::GResult,
    file_system::{DirEntry, Directory, File},
    object_store::{
        index::{FanoutTable, ShortOffsetTable},
        lookup::PackName,
        pack::validate_packfile_version,
    },
};
use alloc::vec::Vec;

#[derive(Clone)]
pub(crate) struct IndexCache {
    pub indexes: Vec<(PackName, FanoutTable, ShortOffsetTable)>,
}

impl IndexCache {
    pub async fn new<F: File, D: Directory<F>>(pack_dir: &D) -> GResult<Self> {
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
        Ok(Self { indexes: fanouts })
    }
}
