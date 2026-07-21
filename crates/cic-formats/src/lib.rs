//! Bounded decoders and immutable format values.

mod csf;
mod w3d;
mod w3d_material;
mod w3d_mesh;
mod w3d_scene;

pub use csf::{CsfError, CsfFile, CsfHeader, CsfLabel, CsfLimits, CsfString, parse_csf};
pub use w3d::{W3dChunk, W3dError, W3dFile, W3dLimits, W3dPayload, parse_w3d, w3d_chunk_name};
pub use w3d_material::{
    W3dFaceIds, W3dMaterialError, W3dMaterialIds, W3dMaterialInfo, W3dMaterialPass, W3dMaterialSet,
    W3dRgb8, W3dRgba8, W3dShader, W3dTexCoord, W3dTexture, W3dTextureInfo, W3dTextureStage,
    W3dVertexMaterial,
};
pub use w3d_mesh::{
    W3dMeshError, W3dMeshHeader3, W3dMeshLimits, W3dStaticMesh, W3dTriangle, W3dVector3,
    decode_static_mesh,
};
pub use w3d_scene::{
    W3dAnimation, W3dAnimationChannel, W3dAnimationChannelKind, W3dHierarchy, W3dHlod, W3dLod,
    W3dModel, W3dModelMesh, W3dPivot, W3dQuaternion, W3dSceneError, W3dSceneLimits, W3dSubObject,
    decode_w3d_model, decode_w3d_model_set, w3d_model_hierarchy_name,
};
