use crate::{
    Repo,
    error::GResult,
    file_system::{DirEntry, Directory},
    object_store::{index::IndexFanout, lookup::Pack},
    repo::RepoCache,
    sync::SharedCell,
    traits::AllGenerics,
};
use alloc::vec::Vec;

pub struct PackFileCache<G: AllGenerics> {
    pub pack_dir: G::Directory,
    pub fanouts: Vec<(Pack, IndexFanout)>,
}

impl<G: AllGenerics> PackFileCache<G> {
    pub async fn new(repo: &Repo<G>) -> GResult<Self> {
        let pack_dir = repo
            .git_dir
            .open_subdir(b"objects")
            .await?
            .open_subdir(b"pack")
            .await?;
        let pack_ids: Vec<Pack> = pack_dir
            .list_dir()
            .await?
            .into_iter()
            .filter_map(|dirent| -> Option<Pack> {
                use DirEntry::*;
                let File(name) = dirent else { None? };
                Pack::new(name)
            })
            .collect();
        let mut fanouts = Vec::with_capacity(pack_ids.len());
        for pack_id in pack_ids {
            let mut file = pack_dir.open_file(&pack_id.index_filename).await?;
            let fanout = IndexFanout::load(&mut file).await?;
            fanouts.push((pack_id, fanout));
        }
        Ok(Self { pack_dir, fanouts })
    }

    pub async fn get_or_init(
        repo: &Repo<G>,
    ) -> GResult<<G::SharedCell<RepoCache<G>> as SharedCell<RepoCache<G>>>::Guard<'_>> {
        let guard = {
            let read_guard = repo.cache.get().await?;
            if read_guard.pack_cache.is_none() {
                drop(read_guard);
                let mut write_guard = repo.cache.get_mut().await?;
                write_guard.pack_cache = Some(PackFileCache::new(repo).await?);
                drop(write_guard);
                repo.cache.get().await?
            } else {
                read_guard
            }
        };
        Ok(guard)
    }
}
