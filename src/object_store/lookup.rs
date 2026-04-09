use crate::{
    directory::Directory,
    error::{GResult, annotate_with_object_id},
    object::ObjectId,
    object_store::{
        ObjectType, RawObject,
        index::find_object,
        loose::{read_loose_object, read_loose_object_size_type},
        pack::{
            PackObject, PackObjectType, form_deltified_chain,
            reconstruct_deltified_object_from_chain,
        },
    },
    repo::Repo,
};
use alloc::vec::Vec;

async fn get_packfile_chain<D: Directory>(
    repo: &Repo<D>,
    id: ObjectId,
) -> GResult<Option<(D::File, Vec<PackObject>, PackObject)>> {
    let pack_file_location = if let Some(location) = find_object(repo, id).await? {
        location
    } else {
        return Ok(None);
    };
    let mut pack_file_name = b"pack-".to_vec();
    pack_file_name.extend_from_slice(&pack_file_location.pack_id);
    pack_file_name.extend_from_slice(b".pack");
    let mut pack_file = repo
        .git_dir
        .open_subdir(b"objects")
        .await?
        .open_subdir(b"pack")
        .await?
        .open_file(&pack_file_name)
        .await?;
    let (chain, final_object) = form_deltified_chain(&mut pack_file, pack_file_location.offset)
        .await
        .map_err(annotate_with_object_id(id))?;
    Ok(Some((pack_file, chain, final_object)))
}

pub(crate) async fn lookup_size_type<D: Directory>(
    repo: &Repo<D>,
    id: ObjectId,
) -> GResult<Option<(u64, ObjectType)>> {
    let opt_size_type = read_loose_object_size_type(repo, id).await?;
    if opt_size_type.is_some() {
        return Ok(opt_size_type);
    }
    let opt_chain = get_packfile_chain(repo, id).await?;
    let (_, _, final_object) = if let Some(pieces) = opt_chain {
        pieces
    } else {
        return Ok(None);
    };
    let object_type = match final_object.object_type {
        PackObjectType::Base(raw_object_type) => raw_object_type,
        PackObjectType::OffsetDelta { .. } => unreachable!(),
        PackObjectType::RefDelta => todo!(),
    };
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
    let opt_chain = get_packfile_chain(repo, id).await?;
    let (mut pack_file, chain, final_object) = if let Some(pieces) = opt_chain {
        pieces
    } else {
        return Ok(None);
    };
    let (object_type, body) =
        reconstruct_deltified_object_from_chain(&mut pack_file, &chain, &final_object)
            .await
            .map_err(annotate_with_object_id(id))?;
    Ok(Some(RawObject {
        object_type,
        id,
        body,
    }))
}
