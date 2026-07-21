//! Bounded decoders and immutable format values.

mod csf;
mod w3d;
mod w3d_material;
mod w3d_mesh;

pub use csf::{CsfError, CsfFile, CsfHeader, CsfLabel, CsfLimits, CsfString, parse_csf};
pub use w3d::{W3dChunk, W3dError, W3dFile, W3dLimits, W3dPayload, parse_w3d, w3d_chunk_name};
pub use w3d_material::{
    W3dMaterialError, W3dMaterialIds, W3dMaterialInfo, W3dMaterialPass, W3dMaterialSet, W3dRgb8,
    W3dRgba8, W3dVertexMaterial,
};
pub use w3d_mesh::{
    W3dMeshError, W3dMeshHeader3, W3dMeshLimits, W3dStaticMesh, W3dTriangle, W3dVector3,
    decode_static_mesh,
};
