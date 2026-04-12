use crate::{
    directory::{DirEntry, Directory},
    error::{GResult, annotate_with_object_id},
    object::ObjectId,
    object_store::{
        ObjectType, RawObject,
        index::find_object_in_pack_index,
        loose::{read_loose_object, read_loose_object_size_type},
        pack::{form_deltified_chain, reconstruct_deltified_object_from_chain},
    },
    repo::Repo,
};
use alloc::vec::Vec;

pub(crate) async fn lookup_size_type<D: Directory>(
    repo: &Repo<D>,
    id: ObjectId,
) -> GResult<Option<(u64, ObjectType)>> {
    let opt_size_type = read_loose_object_size_type(repo, id).await?;
    if opt_size_type.is_some() {
        return Ok(opt_size_type);
    }
    let (mut pack, offset) = if let Some(pieces) = find_packed_object(repo, id).await? {
        pieces
    } else {
        return Ok(None);
    };
    let (_, object_type, final_object) = form_deltified_chain(&mut pack, offset)
        .await
        .map_err(annotate_with_object_id(id))?;
    Ok(Some((final_object.size, object_type)))
}

pub(crate) async fn lookup<D: Directory>(
    repo: &Repo<D>,
    id: ObjectId,
) -> GResult<Option<RawObject>> {
    let loose_object = read_loose_object(repo, id).await?;
    if loose_object.is_some() {
        return Ok(loose_object);
    }
    let (mut indexed_pack, offset) = if let Some(pieces) = find_packed_object(repo, id).await? {
        pieces
    } else {
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

pub(crate) struct IndexedPackFile<F> {
    pub(crate) index: F,
    pub(crate) pack: F,
}

pub(crate) type PackOffset = u64;

pub(crate) async fn find_packed_object<D: Directory>(
    repo: &Repo<D>,
    id: ObjectId,
) -> GResult<Option<(IndexedPackFile<D::File>, PackOffset)>> {
    let pack_dir = repo
        .git_dir
        .open_subdir(b"objects")
        .await?
        .open_subdir(b"pack")
        .await?;
    let idx_filenames: Vec<Vec<u8>> = pack_dir
        .list_dir()
        .await?
        .into_iter()
        .filter_map(|dirent| -> Option<Vec<u8>> {
            use DirEntry::*;
            let name = if let File(name) = dirent {
                Some(name)
            } else {
                None
            }?;
            if name.ends_with(b".idx") {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    for idx in idx_filenames {
        let mut idx_file = pack_dir.open_file(&idx).await?;
        if let Some(offset) = find_object_in_pack_index(&mut idx_file, id).await? {
            let mut pack_file_name = idx.strip_suffix(b".idx").unwrap().to_vec();
            pack_file_name.extend_from_slice(b".pack");
            let pack_file = pack_dir.open_file(&pack_file_name).await?;
            return Ok(Some((
                IndexedPackFile {
                    index: idx_file,
                    pack: pack_file,
                },
                offset,
            )));
        };
    }
    Ok(None)
}
