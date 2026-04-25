use crate::{
    error::GResult,
    file_system::{DirEntry, Directory, File},
    object_store::{
        index::{FanoutTable, ShortOffsetTable},
        lookup::PackName,
        pack::validate_packfile_version,
    },
    repo::RepoConfig,
};
use alloc::vec::Vec;

#[derive(Clone)]
pub(crate) struct IndexCache {
    pub indexes: Vec<(PackName, FanoutTable, Option<ShortOffsetTable>)>,
}

impl IndexCache {
    pub async fn new<F: File, D: Directory<F>>(pack_dir: &D, config: &RepoConfig) -> GResult<Self> {
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
        let mut indexes = Vec::with_capacity(pack_ids.len());
        let mut objects_in_offset_cache: usize = 0;
        let max_objects_in_offset_cache = config.index_offset_cache_max / size_of::<u32>();
        for pack_id in pack_ids {
            let mut file = pack_dir.open_file(&pack_id.index_filename).await?;
            let fanout = FanoutTable::load(&mut file).await?;
            let index_objects: usize = fanout.total_objects().try_into().unwrap();
            let offset_table =
                if objects_in_offset_cache + index_objects < max_objects_in_offset_cache {
                    objects_in_offset_cache += index_objects;
                    Some(ShortOffsetTable::load(&mut file, fanout.total_objects()).await?)
                } else {
                    None
                };
            let mut pack_file = pack_dir.open_file(&pack_id.pack_filename).await?;
            validate_packfile_version(&mut pack_file).await?;
            indexes.push((pack_id, fanout, offset_table));
        }
        Ok(Self { indexes })
    }
}
