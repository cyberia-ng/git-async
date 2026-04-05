use crate::{
    directory::Directory,
    error::GResult,
    object::ObjectId,
    object_store::{
        RawObject,
        index::find_object,
        loose::read_loose_object,
        pack::{
            PackObject, PackObjectType, form_deltified_chain,
            reconstruct_deltified_object_from_chain,
        },
    },
    repo::Repo,
};

pub(crate) async fn lookup<D: Directory>(
    repo: &Repo<D>,
    id: ObjectId,
) -> GResult<Option<RawObject>> {
    let loose_object = read_loose_object(repo, id).await?;
    if loose_object.is_some() {
        return Ok(loose_object);
    }
    let pack_file_location = if let Some(location) = find_object(repo, id).await? {
        location
    } else {
        return Ok(None);
    };
    let mut pack_file = repo
        .git_dir
        .open_subdir(b"objects")
        .await?
        .open_subdir(b"pack")
        .await?
        .open_file(&pack_file_location.pack_file_name)
        .await?;
    let (chain, final_object) =
        form_deltified_chain(&mut pack_file, pack_file_location.offset).await?;
    let PackObject { object_type, body } =
        reconstruct_deltified_object_from_chain(&chain, &final_object);
    match object_type {
        PackObjectType::Base(object_type) => Ok(Some(RawObject {
            object_type,
            id,
            body,
        })),
        PackObjectType::OffsetDelta { base_offset_neg: _ } => unreachable!(),
        PackObjectType::RefDelta => todo!(),
    }
}
