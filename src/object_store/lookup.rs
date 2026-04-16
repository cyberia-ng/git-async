use crate::{
    error::{GResult, annotate_with_object_id},
    file_system::{Directory, Offset},
    object::ObjectId,
    object_store::{
        ObjectSize, ObjectType, RawObject,
        cache::PackFileCache,
        index::{FanoutTable, ShortOffsetTable, find_object_in_pack_index},
        loose::{read_loose_object, read_loose_object_size_type},
        pack::{form_deltified_chain, reconstruct_deltified_object_from_chain},
    },
    repo::Repo,
    traits::AllGenerics,
};
use alloc::vec::Vec;

pub(crate) struct Pack {
    pub(crate) index_filename: Vec<u8>,
    pub(crate) pack_filename: Vec<u8>,
}

impl Pack {
    pub(crate) fn new(filename: Vec<u8>) -> Option<Self> {
        let stripped = filename.strip_suffix(b".idx")?;
        let mut pack_filename = Vec::with_capacity(filename.len() + 1);
        pack_filename.extend_from_slice(stripped);
        pack_filename.extend_from_slice(b".pack");
        Some(Self {
            index_filename: filename,
            pack_filename,
        })
    }
}

pub(crate) struct IndexedPackFile<'f, F> {
    pub(crate) index: F,
    pub(crate) fanout: &'f FanoutTable,
    pub(crate) offsets: &'f ShortOffsetTable,
    pub(crate) pack: F,
}

pub(crate) async fn lookup_size_type<G: AllGenerics>(
    repo: &Repo<G>,
    id: ObjectId,
) -> GResult<Option<(ObjectSize, ObjectType)>> {
    let opt_size_type = read_loose_object_size_type(repo, id).await?;
    if opt_size_type.is_some() {
        return Ok(opt_size_type);
    }
    let guard = PackFileCache::get_or_init(repo).await?;
    let pack_cache = guard.pack_cache.as_ref().unwrap();
    let Some((mut pack, offset)) = find_packed_object(pack_cache, id).await? else {
        return Ok(None);
    };
    let (_, object_type, final_object) = form_deltified_chain(&mut pack, offset)
        .await
        .map_err(annotate_with_object_id(id))?;
    Ok(Some((final_object.size, object_type)))
}

pub(crate) async fn lookup<G: AllGenerics>(
    repo: &Repo<G>,
    id: ObjectId,
) -> GResult<Option<RawObject>> {
    let loose_object = read_loose_object(repo, id).await?;
    if loose_object.is_some() {
        return Ok(loose_object);
    }
    let guard = PackFileCache::get_or_init(repo).await?;
    let pack_cache = guard.pack_cache.as_ref().unwrap();
    let Some((mut indexed_pack, offset)) = find_packed_object(pack_cache, id).await? else {
        return Ok(None);
    };
    let (chain, object_type, final_object) = form_deltified_chain(&mut indexed_pack, offset)
        .await
        .map_err(annotate_with_object_id(id))?;
    let body = reconstruct_deltified_object_from_chain(&mut indexed_pack, &chain, &final_object)
        .await
        .map_err(annotate_with_object_id(id))?;
    Ok(Some(RawObject {
        object_type,
        id,
        body,
    }))
}

pub(crate) async fn find_packed_object<G: AllGenerics>(
    pack_cache: &PackFileCache<G>,
    id: ObjectId,
) -> GResult<Option<(IndexedPackFile<'_, G::File>, Offset)>> {
    for (pack_meta, fanout, offsets) in &pack_cache.indexes {
        let mut idx_file = pack_cache
            .pack_dir
            .open_file(&pack_meta.index_filename)
            .await?;
        if let Some(offset) = find_object_in_pack_index(fanout, offsets, &mut idx_file, id).await? {
            let pack_file = pack_cache
                .pack_dir
                .open_file(&pack_meta.pack_filename)
                .await?;
            return Ok(Some((
                IndexedPackFile {
                    fanout,
                    offsets,
                    index: idx_file,
                    pack: pack_file,
                },
                offset,
            )));
        }
    }
    Ok(None)
}
